//! Credits of a unit sold for money: at the unit's price in the customer's currency, taxed at
//! purchase under its product, the grant costing the money net of tax, and the price pinned when a
//! top-up is queued.
use crate::data::ids::{
    CUST_UBER_ID, CUST_UBER_PAYMENT_METHOD_ID, INVOICING_ENTITY_ID, PRODUCT_FAMILY_ID, TENANT_ID,
    USER_ID,
};
use crate::data::payment::{
    run_customer_payment_methods_seed, run_mock_payment_provider_seed, set_mock_charge_behavior,
};
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use crate::test_customer_credit::pay_amount;
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, CreditUnitId, CustomerId, PaymentTransactionId, ProductId, WalletId,
};
use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use diesel_models::customer_credit::{CreditPurchaseRow, CustomerCreditOperationRow};
use diesel_models::enums::FeeTypeEnum;
use diesel_models::products::ProductRowNew;
use meteroid::workers::pgmq::processors::run_once_invoice_orchestration;
use meteroid_store::domain::InvoicingEntityPatch;
use meteroid_store::domain::connectors::MockChargeBehavior;
use meteroid_store::domain::credit::{
    AutoTopUpChange, ConfigureCreditWallet, CreditGrant, TopUpWallet, UnitPriceChange,
};
use meteroid_store::domain::credit_units::{CreditUnitNew, CreditUnitPatch};
use meteroid_store::domain::enums::{InvoicePaymentStatus, TaxResolverEnum};
use meteroid_store::domain::payment_transactions::PaymentIntent;
use meteroid_store::domain::{Address, CustomerTaxRate, PaymentStatusEnum};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit_notes::{
    CreateCreditNoteParams, CreditLineItem, CreditType,
};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::payment_transactions::PaymentTransactionInterface;
use meteroid_store::repositories::{CreditNoteInterface, CreditUnitsInterface, InvoiceInterface};
use meteroid_store::services::{InvoiceDelivery, SellCustomerCredit, UnitPriceSource};
use rust_decimal_macros::dec;
use std::sync::Arc;
use uuid::Uuid;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn test_error(error: impl std::fmt::Debug) -> Box<dyn std::error::Error> {
    format!("{error:?}").into()
}

const UNIT: &str = "HALYARD";

/// Uber (EUR) taxed 10% by the manual resolver, with saved cards on a mock provider.
pub(crate) async fn setup() -> MeteroidSetup {
    crate::helpers::init::logging();
    let database =
        crate::meteroid_it::container::create_seeded_test_database(SeedLevel::PRODUCT).await;
    let setup = crate::meteroid_it::container::start_meteroid(database, SeedLevel::PRODUCT).await;
    run_mock_payment_provider_seed(&setup.store.pool, false).await;
    run_customer_payment_methods_seed(&setup.store.pool).await;
    setup
        .store
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::Manual),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("manual tax resolver");
    execute(
        &setup,
        format!(
            r#"UPDATE customer
               SET billing_address = '{{"line1":"1 Rue de Rivoli","city":"Paris","country":"FR","zip_code":"75001"}}'::jsonb,
                   custom_taxes = '[{{"tax_code":"vat","name":"VAT","rate":0.1}}]'::jsonb
               WHERE id = '{}'"#,
            CUST_UBER_ID.as_uuid()
        ),
    )
    .await
    .expect("taxable customer");
    setup
}

async fn execute(setup: &MeteroidSetup, sql: String) -> TestResult {
    use diesel_async::SimpleAsyncConnection;
    setup
        .store
        .get_conn()
        .await
        .map_err(test_error)?
        .batch_execute(&sql)
        .await
        .map_err(test_error)
}

async fn drain(setup: &MeteroidSetup) {
    for _ in 0..4 {
        run_once_invoice_orchestration(
            Arc::new(setup.store.clone()),
            Arc::new(setup.services.clone()),
        )
        .await;
    }
}

async fn create_unit(setup: &MeteroidSetup) -> TestResult<CreditUnitId> {
    Ok(setup
        .store
        .create_credit_unit(
            CreditUnitNew::new(
                TENANT_ID,
                UNIT,
                "Halyard credit",
                "Halyard credits",
                None,
                0,
            )
            .map_err(test_error)?,
        )
        .await
        .map_err(test_error)?
        .id)
}

async fn set_product(setup: &MeteroidSetup, unit: CreditUnitId) -> TestResult<ProductId> {
    let product_id = ProductId::new();
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    ProductRowNew {
        id: product_id,
        name: "Halyard credits".to_owned(),
        description: None,
        tenant_id: TENANT_ID,
        product_family_id: PRODUCT_FAMILY_ID,
        fee_type: FeeTypeEnum::Credit,
        fee_structure: serde_json::to_value(
            &meteroid_store::domain::prices::FeeStructure::Credit {},
        )
        .map_err(test_error)?,
        catalog: true,
        tax_category_id: None,
    }
    .insert(&mut conn)
    .await
    .map_err(test_error)?;
    drop(conn);
    setup
        .store
        .update_credit_unit(
            CreditUnitPatch::new(TENANT_ID, unit, None, None, None, None)
                .map_err(test_error)?
                .with_product_id(Some(product_id)),
        )
        .await
        .map_err(test_error)?;
    Ok(product_id)
}

/// A unit sold at €0.001 a credit, with its tax product, and Uber's wallet in it.
pub(crate) async fn sellable(setup: &MeteroidSetup) -> TestResult<(CreditUnitId, WalletId)> {
    let unit = create_unit(setup).await?;
    set_product(setup, unit).await?;
    setup
        .store
        .set_credit_unit_price(TENANT_ID, unit, "EUR", dec!(0.001))
        .await
        .map_err(test_error)?;
    let wallet = setup
        .store
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, UNIT)
        .await
        .map_err(test_error)?
        .id;
    Ok((unit, wallet))
}

async fn unit_grants(setup: &MeteroidSetup) -> TestResult<Vec<CreditGrant>> {
    Ok(setup
        .store
        .customer_credit_wallets(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?
        .into_iter()
        .filter(|wallet| wallet.currency == UNIT)
        .flat_map(|wallet| wallet.grants)
        .collect())
}

async fn operations(setup: &MeteroidSetup) -> TestResult<Vec<CustomerCreditOperationRow>> {
    use diesel_models::schema::customer_credit_operation::dsl as o;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    o::customer_credit_operation
        .filter(o::tenant_id.eq(TENANT_ID))
        .filter(o::customer_id.eq(CUST_UBER_ID))
        .order(o::created_at.asc())
        .select(CustomerCreditOperationRow::as_select())
        .load(&mut conn)
        .await
        .map_err(test_error)
}

pub(crate) fn sell(wallet: WalletId, credits: i64) -> SellCustomerCredit {
    SellCustomerCredit {
        expires_at: None,
        tenant_id: TENANT_ID,
        customer_id: CUST_UBER_ID,
        wallet_id: wallet,
        amount_minor: credits,
        expires_in_months: None,
        note: None,
        actor: Actor::User { id: USER_ID },
        unit_price: None,
        expected_unit_price: None,
    }
}

fn precondition_message(error: &error_stack::Report<StoreError>) -> String {
    match error.current_context() {
        StoreError::FailedPrecondition(message) => message.clone(),
        other => panic!("expected a precondition, got {other:?}"),
    }
}

#[tokio::test]
async fn a_unit_without_a_product_or_a_price_is_not_sold() -> TestResult {
    let setup = setup().await;
    let unit = create_unit(&setup).await?;
    let wallet = setup
        .store
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, UNIT)
        .await
        .map_err(test_error)?
        .id;

    let refused = setup
        .services
        .sell_customer_credit(Uuid::now_v7(), sell(wallet, 50_000))
        .await
        .expect_err("no product");
    assert!(precondition_message(&refused).contains("product"));

    set_product(&setup, unit).await?;
    let refused = setup
        .services
        .quote_wallet_top_up(TENANT_ID, CUST_UBER_ID, wallet, 50_000)
        .await
        .expect_err("no price in EUR");
    assert!(precondition_message(&refused).contains("no selling price in EUR"));

    // A price in another currency does not sell to a EUR customer.
    setup
        .store
        .set_credit_unit_price(TENANT_ID, unit, "USD", dec!(0.001))
        .await
        .map_err(test_error)?;
    assert!(
        setup
            .services
            .sell_customer_credit(Uuid::now_v7(), sell(wallet, 50_000))
            .await
            .is_err()
    );
    assert!(unit_grants(&setup).await?.is_empty());
    Ok(())
}

/// 50,000 credits at €0.001 sell for €50.00 + 10% tax; the grant holds the credits at the net
/// €50.00, and refunding the invoice takes back the credits and the tax.
#[tokio::test]
async fn a_unit_sale_is_taxed_at_purchase_and_its_credit_note_claws_back_units_and_tax()
-> TestResult {
    let setup = setup().await;
    let (_, wallet) = sellable(&setup).await?;

    let quote = setup
        .services
        .quote_credit_sale(TENANT_ID, CUST_UBER_ID, wallet, 50_000, None)
        .await
        .map_err(test_error)?;
    assert_eq!(
        (
            quote.quote.net_minor,
            quote.quote.tax_minor,
            quote.quote.charge_minor
        ),
        (5_000, 500, 5_500)
    );
    assert_eq!(quote.quote.currency, "EUR");

    let purchase_id = Uuid::now_v7();
    let sold = setup
        .services
        .sell_customer_credit(purchase_id, sell(wallet, 50_000))
        .await
        .map_err(test_error)?;
    let invoice = sold.invoice.invoice;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let purchase = CreditPurchaseRow::find_by_id(&mut conn, TENANT_ID, purchase_id)
        .await
        .map_err(test_error)?
        .ok_or("the purchase is recorded")?;
    drop(conn);
    // Money stays money: the credits sold have their own column.
    assert_eq!(
        (purchase.amount_cents, purchase.amount_credits),
        (5_000, Some(50_000))
    );
    assert_eq!(invoice.currency, "EUR");
    assert_eq!(
        (invoice.subtotal, invoice.tax_amount, invoice.total),
        (5_000, 500, 5_500)
    );
    let line = &invoice.line_items[0];
    assert!(line.taxed_at_purchase);
    assert!(line.product_id.is_some());
    assert_eq!(line.credit_grant_amount, Some(50_000));
    assert_eq!(line.unit_price, Some(dec!(0.001)));
    assert_eq!(
        line.credit_grant_unit
            .as_ref()
            .map(|unit| unit.code.as_str()),
        Some(UNIT)
    );
    assert!(unit_grants(&setup).await?.is_empty(), "funded once paid");

    pay_amount(&setup, invoice.id, invoice.amount_due).await?;
    let grant = unit_grants(&setup).await?.remove(0);
    assert_eq!(
        (grant.amount_minor, grant.remaining_minor),
        (50_000, 50_000)
    );
    let cost = grant.cost.as_ref().ok_or("the sold grant has a cost")?;
    assert_eq!((cost.amount_minor, cost.currency.as_str()), (5_000, "EUR"));

    let note = setup
        .store
        .create_and_finalize_credit_note(
            Actor::System,
            TENANT_ID,
            CreateCreditNoteParams {
                invoice_id: invoice.id,
                line_items: vec![CreditLineItem::Amount {
                    local_id: line.local_id.clone(),
                    amount_cents: 5_000,
                }],
                reason: Some("Refund".into()),
                memo: None,
                credit_type: CreditType::Refund,
                custom_properties: None,
            },
        )
        .await
        .map_err(test_error)?;
    assert_eq!((note.tax_amount, note.total), (-500, -5_500));
    let grant = unit_grants(&setup).await?.remove(0);
    assert_eq!(grant.remaining_minor, 0, "every credit is taken back");
    Ok(())
}

/// A card top-up of a unit wallet at an operator's negotiated price charges that price plus tax.
#[tokio::test]
async fn a_unit_top_up_at_a_negotiated_price_charges_it_plus_tax() -> TestResult {
    let setup = setup().await;
    let (_, wallet) = sellable(&setup).await?;
    // The hosted page shows the money: net, tax and total, never the credits.
    let session = setup
        .services
        .create_top_up_checkout(
            TENANT_ID,
            CUST_UBER_ID,
            wallet,
            50_000,
            "hosted".into(),
            None,
            None,
            None,
        )
        .await
        .map_err(test_error)?;
    let hosted = session.top_up().map_err(test_error)?;
    assert_eq!(hosted.currency, "EUR");
    assert_eq!(
        (
            hosted.net_minor(),
            hosted.tax_minor(),
            hosted.charge_minor()
        ),
        (5_000, 500, 5_500)
    );
    let receipt = setup
        .services
        .top_up_wallet(
            TopUpWallet {
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                wallet_id: wallet,
                amount_minor: 12_345,
                payment_method_id: CUST_UBER_PAYMENT_METHOD_ID,
                idempotency_key: "negotiated".to_owned(),
                expires_at: None,
                unit_price: Some(dec!(0.0013)),
                expected_unit_price: None,
            },
            None,
        )
        .await
        .map_err(test_error)?;
    drain(&setup).await;
    // 12,345 × €0.0013 = €16.0485, rounded once to €16.05, plus 10% tax.
    assert_eq!(receipt.currency, "EUR");
    assert_eq!(receipt.credit_minor, 12_345);
    let invoice = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, receipt.invoice_id)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(invoice.subtotal, 1_605);
    assert!((160..=161).contains(&invoice.tax_amount));
    assert_eq!(receipt.charge_minor, invoice.subtotal + invoice.tax_amount);
    assert_eq!(receipt.transaction.amount, receipt.charge_minor);
    let grant = unit_grants(&setup).await?.remove(0);
    assert_eq!(grant.amount_minor, 12_345);
    assert_eq!(
        grant.cost.as_ref().map(|cost| cost.amount_minor),
        Some(1_605)
    );
    let pinned = &operations(&setup).await?[0].metadata["unit_sale"];
    assert_eq!(pinned["unit_price"], "0.0013");
    Ok(())
}

async fn set_tax_status(setup: &MeteroidSetup, status: &str) -> TestResult {
    execute(
        setup,
        format!(
            "UPDATE customer SET tax_status = '{status}' WHERE id = '{}'",
            CUST_UBER_ID.as_uuid()
        ),
    )
    .await
}

fn top_up(wallet: WalletId, credits: i64, key: &str) -> TopUpWallet {
    TopUpWallet {
        tenant_id: TENANT_ID,
        customer_id: CUST_UBER_ID,
        wallet_id: wallet,
        amount_minor: credits,
        payment_method_id: CUST_UBER_PAYMENT_METHOD_ID,
        idempotency_key: key.to_owned(),
        expires_at: None,
        unit_price: None,
        expected_unit_price: None,
    }
}

/// The provider's capture of a charge whose attempt was given up, as its webhook records it.
async fn capture_late(setup: &MeteroidSetup, payment: PaymentTransactionId) -> TestResult {
    let transaction = setup
        .store
        .get_payment_tx_by_id(TENANT_ID, payment)
        .await
        .map_err(test_error)?;
    let intent = PaymentIntent {
        external_id: transaction
            .provider_transaction_id
            .clone()
            .unwrap_or_else(|| format!("late_{}", payment.as_uuid())),
        transaction_id: payment,
        tenant_id: TENANT_ID,
        amount_requested: transaction.amount,
        amount_received: Some(transaction.amount),
        currency: transaction.currency.clone(),
        next_action: None,
        status: PaymentStatusEnum::Settled,
        last_payment_error: None,
        decline_kind: None,
        processed_at: Some(chrono::Utc::now().naive_utc()),
    };
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    setup
        .store
        .consolidate_intent_and_transaction_tx(&mut conn, &Actor::System, transaction, intent)
        .await
        .map_err(test_error)?;
    drop(conn);
    drain(setup).await;
    Ok(())
}

/// The price moves after the hosted page was shown: paying it is refused, nothing is charged.
#[tokio::test]
async fn paying_a_checkout_after_the_price_changed_is_refused() -> TestResult {
    let setup = setup().await;
    let (unit, wallet) = sellable(&setup).await?;
    let session = setup
        .services
        .create_top_up_checkout(
            TENANT_ID,
            CUST_UBER_ID,
            wallet,
            50_000,
            "page".into(),
            None,
            None,
            None,
        )
        .await
        .map_err(test_error)?;
    setup
        .store
        .set_credit_unit_price(TENANT_ID, unit, "EUR", dec!(0.002))
        .await
        .map_err(test_error)?;
    let Err(refused) = setup
        .services
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            5_500,
            "EUR".to_owned(),
            None,
        )
        .await
    else {
        return Err("the page showed the old price: paying must be refused".into());
    };
    assert_eq!(
        precondition_message(&refused),
        "The price changed; refresh before paying."
    );
    assert!(operations(&setup).await?.is_empty(), "nothing was charged");
    assert!(unit_grants(&setup).await?.is_empty());
    Ok(())
}

/// No tax is due from an exempt or a reverse-charge customer: they pay the net, which the grant
/// costs.
#[tokio::test]
async fn exempt_and_reverse_charge_customers_buy_credits_untaxed() -> TestResult {
    let setup = setup().await;
    let (_, wallet) = sellable(&setup).await?;
    for status in ["EXEMPT", "REVERSE_CHARGE"] {
        set_tax_status(&setup, status).await?;
        let sold = setup
            .services
            .sell_customer_credit(Uuid::now_v7(), sell(wallet, 50_000))
            .await
            .map_err(test_error)?;
        let invoice = sold.invoice.invoice;
        assert_eq!(
            (invoice.subtotal, invoice.tax_amount, invoice.total),
            (5_000, 0, 5_000),
            "{status}"
        );
        pay_amount(&setup, invoice.id, invoice.amount_due).await?;
    }
    let grants = unit_grants(&setup).await?;
    assert_eq!(grants.len(), 2);
    for grant in grants {
        assert_eq!(
            grant.cost.as_ref().map(|cost| cost.amount_minor),
            Some(5_000)
        );
    }
    Ok(())
}

async fn customer_in(setup: &MeteroidSetup, currency: &str) -> TestResult<CustomerId> {
    use diesel_models::customers::CustomerRowNew;
    let customer_id = CustomerId::new();
    let address = Address {
        line1: Some("1 Rue de Rivoli".to_owned()),
        line2: None,
        city: Some("Paris".to_owned()),
        country: Some("FR".parse().map_err(test_error)?),
        state: None,
        zip_code: Some("75001".to_owned()),
    };
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    CustomerRowNew {
        id: customer_id,
        name: format!("{currency} buyer"),
        created_at: Some(chrono::Utc::now().naive_utc()),
        tenant_id: TENANT_ID,
        alias: None,
        currency: currency.to_owned(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address: Some(serde_json::to_value(&address).map_err(test_error)?),
        shipping_address: None,
        billing_email: None,
        current_payment_method_id: None,
        vat_number: None,
        legal_number: None,
        buyer_reference: None,
        custom_taxes: serde_json::to_value(vec![CustomerTaxRate {
            tax_code: "vat".to_owned(),
            name: "VAT".to_owned(),
            rate: dec!(0.10),
        }])
        .map_err(test_error)?,
        invoicing_emails: vec![],
        phone: None,
        tax_status: diesel_models::enums::CustomerTaxStatusEnum::Taxable,
        exemption_reason: None,
        vat_number_format_valid: true,
        connected_account_id: None,
        vat_number_validation_status: None,
        custom_properties: serde_json::Value::Null,
        preferred_locales: vec![],
        customer_type: diesel_models::enums::CustomerTypeEnum::Company,
        first_name: None,
        last_name: None,
    }
    .insert(&mut conn)
    .await
    .map_err(test_error)?;
    Ok(customer_id)
}

/// Zero- and three-decimal currencies: the price rounds once, at the currency's precision.
#[tokio::test]
async fn credits_sell_in_zero_and_three_decimal_currencies() -> TestResult {
    let setup = setup().await;
    let unit = create_unit(&setup).await?;
    set_product(&setup, unit).await?;
    for (currency, price, credits, subtotal, tax) in [
        // 333 × ¥1.5 = ¥499.5 → ¥500, tax ¥50.
        ("JPY", dec!(1.5), 333_i64, 500_i64, 50_i64),
        // 70 × BD 0.0005 = BD 0.035, tax BD 0.0035 → 0.004 (half away from zero).
        ("BHD", dec!(0.0005), 70, 35, 4),
    ] {
        setup
            .store
            .set_credit_unit_price(TENANT_ID, unit, currency, price)
            .await
            .map_err(test_error)?;
        let customer = customer_in(&setup, currency).await?;
        let wallet = setup
            .store
            .create_customer_credit_wallet(TENANT_ID, customer, UNIT)
            .await
            .map_err(test_error)?
            .id;
        let sold = setup
            .services
            .sell_customer_credit(
                Uuid::now_v7(),
                SellCustomerCredit {
                    customer_id: customer,
                    ..sell(wallet, credits)
                },
            )
            .await
            .map_err(test_error)?;
        let invoice = sold.invoice.invoice;
        assert_eq!(invoice.currency, currency);
        assert_eq!(
            (invoice.subtotal, invoice.tax_amount, invoice.total),
            (subtotal, tax, subtotal + tax),
            "{currency}"
        );
        assert_eq!(invoice.line_items[0].credit_grant_amount, Some(credits));
    }
    Ok(())
}

/// The same key with another negotiated price is another sale: refused, never the first one.
#[tokio::test]
async fn replaying_a_sale_with_another_price_is_refused() -> TestResult {
    let setup = setup().await;
    let (_, wallet) = sellable(&setup).await?;
    let key = Uuid::now_v7();
    setup
        .services
        .sell_customer_credit(
            key,
            SellCustomerCredit {
                unit_price: Some(dec!(0.0008)),
                expected_unit_price: None,
                ..sell(wallet, 50_000)
            },
        )
        .await
        .map_err(test_error)?;
    let replayed = setup
        .services
        .sell_customer_credit(
            key,
            SellCustomerCredit {
                unit_price: Some(dec!(0.00080)),
                expected_unit_price: None,
                ..sell(wallet, 50_000)
            },
        )
        .await
        .map_err(test_error)?;
    assert_eq!(
        replayed.invoice.invoice.subtotal, 4_000,
        "the same price replays"
    );
    let refused = setup
        .services
        .sell_customer_credit(key, sell(wallet, 50_000))
        .await
        .expect_err("another price");
    assert!(matches!(refused.current_context(), StoreError::Conflict(_)));
    Ok(())
}

/// A capture that arrives after the attempt failed reissues the document at the pinned charge,
/// even once the customer became reverse-charged: the captured €55.00 buys the 50,000 credits.
#[tokio::test]
async fn a_late_capture_of_a_unit_sale_reissues_its_document_at_the_pinned_charge() -> TestResult {
    let setup = setup().await;
    let (_, wallet) = sellable(&setup).await?;
    set_mock_charge_behavior(&setup.store.pool, MockChargeBehavior::Failed).await;
    let declined = setup
        .services
        .top_up_wallet(top_up(wallet, 50_000, "late"), None)
        .await
        .expect_err("declined");
    assert!(matches!(
        declined.current_context(),
        StoreError::FailedPrecondition(_)
    ));
    let failed = operations(&setup).await?.remove(0);
    assert_eq!(failed.metadata["unit_sale"]["total_minor"], 5_500);
    let payment = failed.payment_transaction_id.ok_or("no payment")?;

    set_tax_status(&setup, "REVERSE_CHARGE").await?;
    capture_late(&setup, payment).await?;
    let settled = operations(&setup).await?.remove(0);
    assert_eq!(settled.state, "SETTLED");
    let invoice = setup
        .store
        .get_detailed_invoice_by_id(TENANT_ID, settled.invoice_id.ok_or("no document")?)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!((invoice.total, invoice.tax_amount), (5_500, 0));
    assert_eq!(invoice.payment_status, InvoicePaymentStatus::Paid);
    let grant = unit_grants(&setup).await?.remove(0);
    assert_eq!(grant.amount_minor, 50_000);
    assert_eq!(
        grant.cost.as_ref().map(|cost| cost.amount_minor),
        Some(5_500)
    );
    Ok(())
}

/// A unit sale recorded at a past moment is invoiced then, and its credits are funded from when
/// the payment landed, priced and taxed like a sale made now.
#[tokio::test]
async fn a_backdated_unit_sale_funds_its_credits_from_the_payment() -> TestResult {
    let setup = setup().await;
    let (_, wallet) = sellable(&setup).await?;
    let at = |days: i64| {
        (chrono::Utc::now() - chrono::Duration::days(days))
            .date_naive()
            .and_hms_opt(9, 30, 0)
            .expect("valid time")
            .and_utc()
    };
    let (sold_at, paid_at) = (at(10), at(9));

    let sold = setup
        .services
        .sell_customer_credit_at(
            Uuid::now_v7(),
            sell(wallet, 50_000),
            sold_at,
            InvoiceDelivery::Normal,
        )
        .await
        .map_err(test_error)?;
    let invoice = sold.invoice.invoice;
    assert_eq!(invoice.invoice_date, sold_at.date_naive());
    assert_eq!(invoice.finalized_at, Some(sold_at.naive_utc()));
    assert_eq!(
        (invoice.subtotal, invoice.tax_amount, invoice.total),
        (5_000, 500, 5_500)
    );
    assert!(unit_grants(&setup).await?.is_empty(), "funded once paid");

    let paid = setup
        .services
        .settle_invoice_payment_at(
            Actor::System,
            TENANT_ID,
            invoice.id,
            paid_at,
            Some("wire-1".to_owned()),
            InvoiceDelivery::Normal,
        )
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(paid.payment_status, InvoicePaymentStatus::Paid);

    let grants = unit_grants(&setup).await?;
    assert_eq!(grants.len(), 1);
    assert_eq!(
        (grants[0].amount_minor, grants[0].remaining_minor),
        (50_000, 50_000)
    );
    assert_eq!(grants[0].effective_at, paid_at);
    let (entries, _) = setup
        .store
        .customer_credit_ledger(TENANT_ID, wallet, 10, 0)
        .await
        .map_err(test_error)?;
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].amount_minor, 50_000);
    assert_eq!(entries[0].effective_at, paid_at, "recorded at the payment");
    Ok(())
}

async fn set_agreed_price(
    setup: &MeteroidSetup,
    wallet: WalletId,
    unit_price: UnitPriceChange,
) -> TestResult {
    setup
        .services
        .configure_credit_wallet(ConfigureCreditWallet {
            tenant_id: TENANT_ID,
            customer_id: CUST_UBER_ID,
            wallet_id: wallet,
            auto_top_up: AutoTopUpChange::Keep,
            low_balance_thresholds_minor: None,
            usage_at_zero: None,
            unit_price,
            actor_id: Some(USER_ID.as_uuid()),
            consent_attested_by: Some(common_domain::actor::Actor::System),
        })
        .await
        .map(|_| ())
        .map_err(test_error)
}

async fn purchase_source(setup: &MeteroidSetup, purchase_id: Uuid) -> TestResult<Option<String>> {
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    Ok(
        CreditPurchaseRow::find_by_id(&mut conn, TENANT_ID, purchase_id)
            .await
            .map_err(test_error)?
            .ok_or("the purchase is recorded")?
            .unit_price_source,
    )
}

/// A customer's agreed €0.0008 (list €0.001) prices every purchase that names no price of its
/// own: an operator's sale and a portal top-up, taxed on what is charged. A price given for one
/// purchase still wins.
#[tokio::test]
async fn the_customers_agreed_price_prices_purchases_and_top_ups() -> TestResult {
    let setup = setup().await;
    let (_, wallet) = sellable(&setup).await?;
    set_agreed_price(&setup, wallet, UnitPriceChange::Set(dec!(0.00080))).await?;
    let stored = setup
        .store
        .customer_credit_wallet(TENANT_ID, wallet)
        .await
        .map_err(test_error)?
        .ok_or("the wallet")?
        .unit_price
        .ok_or("the agreed price is stored")?;
    assert_eq!(
        (stored.unit_price, stored.currency.as_str()),
        (dec!(0.0008), "EUR")
    );

    // Operator: quote and invoice at €0.0008, 10% tax on the €40.00 charged.
    let quote = setup
        .services
        .quote_credit_sale(TENANT_ID, CUST_UBER_ID, wallet, 50_000, None)
        .await
        .map_err(test_error)?;
    assert_eq!(
        (
            quote.quote.net_minor,
            quote.quote.tax_minor,
            quote.quote.charge_minor
        ),
        (4_000, 400, 4_400)
    );
    assert_eq!(quote.list_unit_price, Some(dec!(0.001)));
    assert_eq!(quote.unit_price, Some(dec!(0.0008)));
    assert_eq!(quote.unit_price_source, Some(UnitPriceSource::Customer));
    let purchase_id = Uuid::now_v7();
    let sold = setup
        .services
        .sell_customer_credit(purchase_id, sell(wallet, 50_000))
        .await
        .map_err(test_error)?
        .invoice
        .invoice;
    assert_eq!(
        (sold.subtotal, sold.tax_amount, sold.total),
        (4_000, 400, 4_400)
    );
    assert_eq!(sold.line_items[0].unit_price, Some(dec!(0.0008)));
    assert_eq!(
        purchase_source(&setup, purchase_id).await?.as_deref(),
        Some("CUSTOMER")
    );

    // A price given for one purchase wins over the agreed one.
    let override_id = Uuid::now_v7();
    let overridden = setup
        .services
        .sell_customer_credit(
            override_id,
            SellCustomerCredit {
                unit_price: Some(dec!(0.0013)),
                expected_unit_price: None,
                ..sell(wallet, 50_000)
            },
        )
        .await
        .map_err(test_error)?
        .invoice
        .invoice;
    assert_eq!((overridden.subtotal, overridden.tax_amount), (6_500, 650));
    assert_eq!(
        purchase_source(&setup, override_id).await?.as_deref(),
        Some("OVERRIDE")
    );

    // Portal: the price shown and the hosted page charge the agreed price.
    let shown = setup
        .services
        .sellable_unit_prices(TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?
        .remove(&wallet)
        .ok_or("the unit is sellable")?;
    assert_eq!(shown.unit_price, dec!(0.0008));
    let hosted = setup
        .services
        .create_top_up_checkout(
            TENANT_ID,
            CUST_UBER_ID,
            wallet,
            50_000,
            "portal-agreed".into(),
            None,
            None,
            None,
        )
        .await
        .map_err(test_error)?
        .top_up()
        .map_err(test_error)?;
    assert_eq!(
        (
            hosted.net_minor(),
            hosted.tax_minor(),
            hosted.charge_minor()
        ),
        (4_000, 400, 4_400)
    );
    let receipt = setup
        .services
        .top_up_wallet(top_up(wallet, 50_000, "portal-card"), None)
        .await
        .map_err(test_error)?;
    assert_eq!(receipt.charge_minor, 4_400);
    let pinned = operations(&setup).await?.remove(0).metadata["unit_sale"].clone();
    assert_eq!(
        (&pinned["unit_price"], &pinned["source"]),
        (&serde_json::json!("0.0008"), &serde_json::json!("CUSTOMER"))
    );
    Ok(())
}

/// A money wallet has no price per credit: its credit is sold at face value.
#[tokio::test]
async fn a_money_wallet_refuses_an_agreed_price() -> TestResult {
    let setup = setup().await;
    let wallet = setup
        .store
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "EUR")
        .await
        .map_err(test_error)?
        .id;
    let refused = set_agreed_price(&setup, wallet, UnitPriceChange::Set(dec!(0.9)))
        .await
        .expect_err("money credit has no unit price");
    assert!(refused.to_string().contains("credit unit wallets only"));
    Ok(())
}

/// A hosted page opened at the list price, then the customer's agreed price set: paying the page
/// is refused and nothing is charged.
#[tokio::test]
async fn paying_a_checkout_after_the_agreed_price_changed_is_refused() -> TestResult {
    let setup = setup().await;
    let (_, wallet) = sellable(&setup).await?;
    let session = setup
        .services
        .create_top_up_checkout(
            TENANT_ID,
            CUST_UBER_ID,
            wallet,
            50_000,
            "page-agreed".into(),
            None,
            None,
            None,
        )
        .await
        .map_err(test_error)?;
    set_agreed_price(&setup, wallet, UnitPriceChange::Set(dec!(0.0008))).await?;
    let Err(refused) = setup
        .services
        .complete_checkout(
            TENANT_ID,
            session.id,
            CUST_UBER_PAYMENT_METHOD_ID,
            5_500,
            "EUR".to_owned(),
            None,
        )
        .await
    else {
        return Err("the page showed the list price: paying must be refused".into());
    };
    assert_eq!(
        precondition_message(&refused),
        "The price changed; refresh before paying."
    );
    assert!(operations(&setup).await?.is_empty(), "nothing was charged");
    assert!(unit_grants(&setup).await?.is_empty());
    Ok(())
}

/// A portal top-up confirmed at the quote it was shown, after the agreed price changed: refused
/// before anything is charged. An operator's sale and card top-up at a stale price are refused
/// the same way.
#[tokio::test]
async fn a_purchase_at_a_price_no_longer_current_is_refused() -> TestResult {
    let setup = setup().await;
    let (_, wallet) = sellable(&setup).await?;
    let shown = setup
        .services
        .quote_wallet_top_up(TENANT_ID, CUST_UBER_ID, wallet, 50_000)
        .await
        .map_err(test_error)?;
    assert_eq!(shown.charge_minor, 5_500);
    set_agreed_price(&setup, wallet, UnitPriceChange::Set(dec!(0.0008))).await?;
    let refused = setup
        .services
        .top_up_wallet(top_up(wallet, 50_000, "portal-stale"), Some(shown))
        .await
        .expect_err("the quote shown is stale");
    assert_eq!(
        precondition_message(&refused),
        "The price changed; refresh before paying."
    );

    let refused = setup
        .services
        .sell_customer_credit(
            Uuid::now_v7(),
            SellCustomerCredit {
                expected_unit_price: Some(dec!(0.001)),
                ..sell(wallet, 50_000)
            },
        )
        .await
        .expect_err("the operator saw the list price");
    assert_eq!(
        precondition_message(&refused),
        "The price changed; refresh before paying."
    );
    let refused = setup
        .services
        .top_up_wallet(
            TopUpWallet {
                expected_unit_price: Some(dec!(0.001)),
                ..top_up(wallet, 50_000, "admin-stale")
            },
            None,
        )
        .await
        .expect_err("the operator saw the list price");
    assert_eq!(
        precondition_message(&refused),
        "The price changed; refresh before paying."
    );
    assert!(
        operations(&setup)
            .await?
            .iter()
            .all(|operation| operation.payment_transaction_id.is_none()),
        "nothing was charged"
    );

    // At the price shown, the sale goes through.
    let sold = setup
        .services
        .sell_customer_credit(
            Uuid::now_v7(),
            SellCustomerCredit {
                expected_unit_price: Some(dec!(0.00080)),
                ..sell(wallet, 50_000)
            },
        )
        .await
        .map_err(test_error)?;
    assert_eq!(sold.invoice.invoice.subtotal, 4_000);
    Ok(())
}

/// A 12-decimal agreed price in a zero-decimal currency: 1,000,000 credits at ¥0.123456789012 is
/// ¥123,456.789012, rounded once to ¥123,457, plus 10% tax rounded half up to ¥12,346.
#[tokio::test]
async fn a_twelve_decimal_agreed_price_in_yen_is_exact() -> TestResult {
    let setup = setup().await;
    let unit = create_unit(&setup).await?;
    set_product(&setup, unit).await?;
    setup
        .store
        .set_credit_unit_price(TENANT_ID, unit, "JPY", dec!(1.5))
        .await
        .map_err(test_error)?;
    let customer = customer_in(&setup, "JPY").await?;
    let wallet = setup
        .store
        .create_customer_credit_wallet(TENANT_ID, customer, UNIT)
        .await
        .map_err(test_error)?
        .id;
    setup
        .services
        .configure_credit_wallet(ConfigureCreditWallet {
            tenant_id: TENANT_ID,
            customer_id: customer,
            wallet_id: wallet,
            auto_top_up: AutoTopUpChange::Keep,
            low_balance_thresholds_minor: None,
            usage_at_zero: None,
            unit_price: UnitPriceChange::Set(dec!(0.123456789012)),
            actor_id: Some(USER_ID.as_uuid()),
            consent_attested_by: Some(common_domain::actor::Actor::System),
        })
        .await
        .map_err(test_error)?;
    let quote = setup
        .services
        .quote_credit_sale(TENANT_ID, customer, wallet, 1_000_000, None)
        .await
        .map_err(test_error)?;
    assert_eq!(
        (
            quote.quote.net_minor,
            quote.quote.tax_minor,
            quote.quote.charge_minor
        ),
        (123_457, 12_346, 135_803)
    );
    let sold = setup
        .services
        .sell_customer_credit(
            Uuid::now_v7(),
            SellCustomerCredit {
                customer_id: customer,
                ..sell(wallet, 1_000_000)
            },
        )
        .await
        .map_err(test_error)?
        .invoice
        .invoice;
    assert_eq!(
        (sold.subtotal, sold.tax_amount, sold.total),
        (123_457, 12_346, 135_803)
    );
    assert_eq!(sold.line_items[0].unit_price, Some(dec!(0.123456789012)));
    Ok(())
}
