//! What credit leaves on a document: promotional credit on an untaxed tax-inclusive invoice draws
//! what the line costs, once; a credit purchase and a line promotional credit paid whole still
//! have their rows in the VAT breakdown, so the e-invoice states every line's category.
use crate::data::ids::{INVOICING_ENTITY_ID, PLAN_VERSION_1_LEETCODE_ID, TENANT_ID, USER_ID};
use crate::helpers::credit::{Grant, fund_promotional_grant, grant_remaining};
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use chrono::NaiveDate;
use common_domain::actor::Actor;
use common_domain::country::CountryCode;
use common_domain::ids::{CreditGrantId, CustomerId, InvoiceId};
use diesel_models::enums::CustomerTaxStatusEnum;
use meteroid::services::invoice_rendering::InvoicePreviewRenderingService;
use meteroid_store::domain::enums::{InvoicePaymentStatus, InvoiceStatusEnum, InvoiceType};
use meteroid_store::domain::{
    CreateSubscription, EInvoicingFormat, Invoice, InvoiceNew, InvoicingEntityPatch,
    SubscriptionActivationCondition, SubscriptionNew,
};
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::customers::CustomersInterfaceAuto;
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use meteroid_store::repositories::{InvoiceInterface, SubscriptionInterface};
use meteroid_store::services::SellCustomerCredit;
use meteroid_store::services::invoice_lines::invoice_lines::ComputedInvoiceContent;
use rust_decimal_macros::dec;
use std::str::FromStr;
use std::sync::Arc;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn test_error(error: impl std::fmt::Debug) -> Box<dyn std::error::Error> {
    format!("{error:?}").into()
}

async fn setup() -> MeteroidSetup {
    crate::helpers::init::logging();
    let database =
        crate::meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    crate::meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await
}

/// Taxes the seeded French entity's invoices under the EU VAT engine and issues them as Factur-X,
/// with the VAT number every taxed or zero-rated document names (BR-S-02, BR-Z-02).
async fn tax_and_einvoice(setup: &MeteroidSetup) -> TestResult {
    setup
        .store
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(meteroid_store::domain::enums::TaxResolverEnum::MeteroidEuVat),
                einvoicing_format: Some(Some(EInvoicingFormat::FacturX)),
                vat_number: Some("FR12889123456".to_owned()),
                legal_number: Some(Some("88912345600015".to_owned())),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .map_err(test_error)?;
    Ok(())
}

/// A VAT-registered business of `country`, or a customer no tax engine can reach.
async fn customer(setup: &MeteroidSetup, country: Option<&str>) -> TestResult<CustomerId> {
    use diesel_models::customers::CustomerRowNew;
    use meteroid_store::domain::Address;
    let id = CustomerId::from_proto(uuid::Uuid::new_v4().to_string()).map_err(test_error)?;
    let billing_address = country
        .map(|country| {
            serde_json::to_value(Address {
                line1: Some("1 Rue de la Paix".to_string()),
                line2: None,
                city: Some("Paris".to_string()),
                country: CountryCode::from_str(country).ok(),
                state: None,
                zip_code: Some("75002".to_string()),
            })
        })
        .transpose()?;
    CustomerRowNew {
        id,
        name: "Client SARL".to_string(),
        created_at: Some(chrono::Utc::now().naive_utc()),
        tenant_id: TENANT_ID,
        alias: None,
        currency: "EUR".to_string(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address,
        shipping_address: None,
        billing_email: Some("compta@client.fr".to_string()),
        current_payment_method_id: None,
        vat_number: country.map(|country| match country {
            "DE" => "DE123456789".to_string(),
            _ => "FR40552100554".to_string(),
        }),
        legal_number: (country == Some("FR")).then(|| "55210055400023".to_string()),
        buyer_reference: None,
        custom_taxes: serde_json::json!([]),
        invoicing_emails: vec![],
        phone: None,
        tax_status: CustomerTaxStatusEnum::Taxable,
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
    .insert(&mut setup.store.get_conn().await.map_err(test_error)?)
    .await
    .map_err(test_error)?;
    Ok(id)
}

/// The LeetCode plan's second 35.00 period, computed as its draft is, with `promotional` credit
/// granted after the first period's invoice (which would draw it), quoted tax-included or not.
async fn period_content(
    setup: &MeteroidSetup,
    customer_id: CustomerId,
    promotional: i64,
    tax_inclusive: bool,
) -> TestResult<(ComputedInvoiceContent, CreditGrantId)> {
    let start = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let subscription_id = setup
        .services
        .insert_subscription(
            Actor::System,
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: start,
                    end_date: None,
                    billing_start_date: Some(start),
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
                    usage_filter: None,
                    custom_properties: serde_json::Value::Null,
                },
                price_components: None,
                add_ons: None,
                coupons: None,
                entitlements: vec![],
                minimum_override: None,
                drop_minimum: false,
            },
            TENANT_ID,
        )
        .await
        .map_err(test_error)?
        .id;
    let mut details = setup
        .store
        .get_subscription_details_with_conn(
            &mut setup.store.get_conn().await.map_err(test_error)?,
            TENANT_ID,
            subscription_id,
        )
        .await
        .map_err(test_error)?;
    details.subscription.tax_inclusive = tax_inclusive;
    let grant = fund_promotional_grant(
        &setup.store,
        TENANT_ID,
        customer_id,
        None,
        Grant::invoice(promotional),
    )
    .await;
    let content = setup
        .services
        .compute_invoice(
            &NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            &details,
            None,
            InvoiceType::Recurring,
        )
        .await
        .map_err(test_error)?;
    Ok((content, grant))
}

/// `content` stored as a draft, as the period's draft is.
async fn draft(
    setup: &MeteroidSetup,
    customer_id: CustomerId,
    content: &ComputedInvoiceContent,
    tax_inclusive: bool,
) -> TestResult<Invoice> {
    let stored = setup
        .store
        .find_customer_by_id(customer_id, TENANT_ID)
        .await
        .map_err(test_error)?;
    let entity = setup
        .store
        .get_invoicing_entity(TENANT_ID, Some(INVOICING_ENTITY_ID))
        .await
        .map_err(test_error)?;
    let today = chrono::Utc::now().date_naive();
    setup
        .store
        .insert_invoice(InvoiceNew {
            status: InvoiceStatusEnum::Draft,
            tenant_id: TENANT_ID,
            customer_id,
            subscription_id: None,
            cycle_index: None,
            currency: "EUR".to_owned(),
            invoice_number: "draft".to_owned(),
            line_items: content.invoice_lines.clone(),
            coupons: Vec::new(),
            data_updated_at: None,
            invoice_date: today,
            billing_period_start: today,
            tax_inclusive,
            plan_version_id: None,
            invoice_type: InvoiceType::OneOff,
            finalized_at: None,
            subtotal: content.subtotal,
            subtotal_recurring: content.subtotal_recurring,
            discount: content.discount,
            tax_amount: content.tax_amount,
            total: content.total,
            amount_due: content.amount_due,
            applied_credits: 0,
            promotional_credits: content.promotional_credits,
            net_terms: 0,
            reference: None,
            purchase_order: None,
            memo: None,
            due_at: None,
            plan_name: None,
            customer_details: stored.into(),
            seller_details: entity.into(),
            auto_advance: false,
            payment_status: InvoicePaymentStatus::Unpaid,
            tax_breakdown: content.tax_breakdown.clone(),
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

/// The e-invoicing findings about the VAT breakdown and the line categories, which is what a
/// receiver rejects a document for when a category has no row of its own.
async fn vat_findings(setup: &MeteroidSetup, invoice_id: InvoiceId) -> TestResult<Vec<String>> {
    let rendering = InvoicePreviewRenderingService::try_new(
        Arc::new(setup.store.clone()),
        setup.object_store.clone(),
        "http://localhost".to_owned(),
    )
    .map_err(test_error)?;
    let report = rendering
        .check_einvoicing_readiness(invoice_id, TENANT_ID)
        .await
        .map_err(test_error)?
        .ok_or("the entity issues e-invoices")?;
    assert_eq!(report.error, None, "the invoice maps onto EN 16931");
    const VAT_RULES: [&str; 8] = [
        "BR-CO-17", "BR-CO-18", "BR-S-", "BR-Z-", "BR-AE-", "BR-E-", "BR-O-", "BR-45",
    ];
    Ok(report
        .findings
        .into_iter()
        .filter(|finding| VAT_RULES.iter().any(|rule| finding.rule.starts_with(rule)))
        .map(|finding| format!("{} at {}: {}", finding.rule, finding.term, finding.message))
        .collect())
}

/// No tax engine reaches a customer without an address: a tax-inclusive line keeps what it costs
/// after its promotional credit, so finalizing draws the credit the line takes, once. 50.00 of
/// promotional credit on a 35.00 line pays 35.00 and leaves 15.00; it drew 50.00 (35.00 counted
/// twice) and billed 20.00 of cash.
#[tokio::test]
async fn untaxed_tax_inclusive_lines_draw_their_promotional_credit_once() -> TestResult {
    let setup = setup().await;
    let customer_id = customer(&setup, None).await?;
    let (content, grant) = period_content(&setup, customer_id, 5_000, true).await?;
    assert_eq!(content.promotional_credits, 3_500);
    assert_eq!((content.total, content.amount_due), (0, 0));
    assert_eq!(content.invoice_lines[0].amount_total, 0);

    let draft = draft(&setup, customer_id, &content, true).await?;
    let finalized = setup
        .services
        .finalize_invoice(Actor::System, draft.id, TENANT_ID)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(finalized.promotional_credits, 3_500);
    assert_eq!((finalized.total, finalized.amount_due), (0, 0));
    assert_eq!(
        grant_remaining(&setup.store, TENANT_ID, customer_id, grant).await,
        1_500,
        "the line's 35.00, drawn once"
    );
    Ok(())
}

/// Currency credit is taxed when it is used, so a top-up's line is left out of the tax pass; the
/// breakdown still states it at a zero rate, or the e-invoice has a line with no category row
/// (BR-CO-18) and is rejected.
#[tokio::test]
async fn a_credit_purchase_has_its_row_in_the_vat_breakdown() -> TestResult {
    let setup = setup().await;
    tax_and_einvoice(&setup).await?;
    let customer_id = customer(&setup, Some("FR")).await?;
    let wallet_id = setup
        .store
        .create_customer_credit_wallet(TENANT_ID, customer_id, "EUR")
        .await
        .map_err(test_error)?
        .id;
    let sold = setup
        .services
        .sell_customer_credit(
            uuid::Uuid::now_v7(),
            SellCustomerCredit {
                expires_at: None,
                tenant_id: TENANT_ID,
                customer_id,
                wallet_id,
                amount_minor: 5_000,
                expires_in_months: None,
                note: None,
                actor: Actor::User { id: USER_ID },
                unit_price: None,
                expected_unit_price: None,
            },
        )
        .await
        .map_err(test_error)?;
    let invoice = sold.invoice.invoice;
    assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
    assert_eq!(invoice.tax_amount, 0, "a prepayment, taxed when used");
    let zero_rated = invoice
        .tax_breakdown
        .iter()
        .find(|row| row.tax_rate.is_zero())
        .ok_or("the purchase has its row")?;
    assert_eq!(
        (zero_rated.taxable_amount, zero_rated.tax_amount),
        (5_000, 0)
    );
    let findings = vat_findings(&setup, invoice.id).await?;
    assert!(findings.is_empty(), "{findings:#?}");
    Ok(())
}

/// A line promotional credit pays whole once the draft is finalized (more credit was granted in
/// between) keeps its 20% rate, and its row stays in the breakdown at nothing (BR-S-01).
#[tokio::test]
async fn a_line_paid_whole_by_promotion_at_finalization_keeps_its_vat_row() -> TestResult {
    let setup = setup().await;
    tax_and_einvoice(&setup).await?;
    let customer_id = customer(&setup, Some("FR")).await?;
    let (content, _) = period_content(&setup, customer_id, 2_000, false).await?;
    assert_eq!(content.promotional_credits, 2_000);
    assert_eq!(content.tax_amount, 300, "20% of the 15.00 left");
    let draft = draft(&setup, customer_id, &content, false).await?;
    fund_promotional_grant(
        &setup.store,
        TENANT_ID,
        customer_id,
        None,
        Grant::invoice(2_000),
    )
    .await;

    let finalized = setup
        .services
        .finalize_invoice(Actor::System, draft.id, TENANT_ID)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(finalized.promotional_credits, 3_500);
    assert_eq!((finalized.tax_amount, finalized.total), (0, 0));
    assert_eq!(finalized.line_items[0].tax_rate, dec!(0.2));
    let row = finalized
        .tax_breakdown
        .iter()
        .find(|row| row.tax_rate == dec!(0.2))
        .ok_or("the 20% row stays")?;
    assert_eq!((row.taxable_amount, row.tax_amount), (0, 0));
    let findings = vat_findings(&setup, finalized.id).await?;
    assert!(findings.is_empty(), "{findings:#?}");
    Ok(())
}

/// A one-off invoice of a 10.00 subscription fee and a 5.00 wallet top-up for `customer_id`,
/// computed as issued and finalized.
async fn fee_and_top_up(setup: &MeteroidSetup, customer_id: CustomerId) -> TestResult<Invoice> {
    use meteroid_store::domain::LineItemKind;
    let fee = crate::test_customer_credit::line("Subscription fee", 1_000, false);
    let mut top_up = crate::test_customer_credit::line("Wallet top-up", 500, false);
    top_up.kind = LineItemKind::Credit;
    top_up.credit_grant_amount = Some(500);
    let content = oneoff_content(setup, customer_id, vec![fee, top_up]).await?;
    let draft = draft(setup, customer_id, &content, false).await?;
    Ok(setup
        .services
        .finalize_invoice(Actor::System, draft.id, TENANT_ID)
        .await
        .map_err(test_error)?
        .invoice)
}

async fn oneoff_content(
    setup: &MeteroidSetup,
    customer_id: CustomerId,
    lines: Vec<meteroid_store::domain::LineItem>,
) -> TestResult<ComputedInvoiceContent> {
    let customer = setup
        .store
        .find_customer_by_id(customer_id, TENANT_ID)
        .await
        .map_err(test_error)?;
    let entity = setup
        .store
        .get_invoicing_entity(TENANT_ID, Some(INVOICING_ENTITY_ID))
        .await
        .map_err(test_error)?;
    setup
        .services
        .compute_oneoff_invoice_for_test(
            chrono::Utc::now().date_naive(),
            lines,
            &entity,
            &customer,
            "EUR".to_owned(),
        )
        .await
        .map_err(test_error)
}

/// Sold to an EU business, the fee is reverse-charged and the top-up is a prepayment taxed when
/// used: its row is apart from the fee's, flagged as credit sold so tax reporting can leave it
/// out. The e-invoice still states one AE group of 15.00.
#[tokio::test]
async fn a_top_up_sold_with_a_reverse_charged_fee_keeps_a_row_of_its_own() -> TestResult {
    use meteroid_store::domain::TaxExemptionType;
    let setup = setup().await;
    tax_and_einvoice(&setup).await?;
    let customer_id = customer(&setup, Some("DE")).await?;
    let invoice = fee_and_top_up(&setup, customer_id).await?;
    assert_eq!(invoice.tax_amount, 0);
    let mut rows = invoice
        .tax_breakdown
        .iter()
        .map(|row| {
            (
                row.taxable_amount,
                row.exemption_type.clone(),
                row.prepaid_credit,
            )
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| row.0);
    assert_eq!(
        rows,
        vec![
            (500, Some(TaxExemptionType::ReverseCharge), true),
            (1_000, Some(TaxExemptionType::ReverseCharge), false),
        ],
        "the top-up keeps the document's category in a row of its own"
    );
    let findings = vat_findings(&setup, invoice.id).await?;
    assert!(findings.is_empty(), "{findings:#?}");
    Ok(())
}

/// Sold to a French business, the fee bears 20% and the top-up nothing, in a zero-rated row of
/// its own flagged as credit sold.
#[tokio::test]
async fn a_domestic_top_up_has_a_zero_rated_row_of_its_own() -> TestResult {
    let setup = setup().await;
    tax_and_einvoice(&setup).await?;
    let customer_id = customer(&setup, Some("FR")).await?;
    let invoice = fee_and_top_up(&setup, customer_id).await?;
    assert_eq!(invoice.tax_amount, 200);
    let sold = invoice
        .tax_breakdown
        .iter()
        .find(|row| row.prepaid_credit)
        .ok_or("the top-up has its own row")?;
    assert_eq!(
        (
            sold.taxable_amount,
            sold.tax_rate,
            sold.exemption_type.clone()
        ),
        (500, rust_decimal::Decimal::ZERO, None)
    );
    let fee = invoice
        .tax_breakdown
        .iter()
        .find(|row| !row.prepaid_credit)
        .ok_or("the fee has its own row")?;
    assert_eq!(
        (fee.taxable_amount, fee.tax_rate, fee.tax_amount),
        (1_000, dec!(0.2), 200)
    );
    let findings = vat_findings(&setup, invoice.id).await?;
    assert!(findings.is_empty(), "{findings:#?}");
    Ok(())
}

/// A standard line and an exempt one (a product of the non-taxable category), both taken whole by
/// promotional credit granted after the draft: rebuilt at finalization, the 20% row stays standard
/// at nothing and only the 0% row carries the exemption (an exempt 20% row breaks BR-E-05).
#[tokio::test]
async fn an_exemption_never_lands_on_a_rated_row_paid_whole_by_promotion() -> TestResult {
    use crate::data::ids::PRODUCT_PLATFORM_FEE_ID;
    use meteroid_store::domain::TaxExemptionType;
    use meteroid_store::repositories::products::{ProductInterface, ProductUpdate};
    use meteroid_store::repositories::tax_categories::TaxCategoryInterface;
    let setup = setup().await;
    tax_and_einvoice(&setup).await?;
    let nontaxable = setup
        .store
        .list_tax_categories(TENANT_ID)
        .await
        .map_err(test_error)?
        .into_iter()
        .find(|category| category.key == "nontaxable")
        .ok_or("the built-in non-taxable category")?;
    setup
        .store
        .update_product(
            Actor::System,
            ProductUpdate {
                id: PRODUCT_PLATFORM_FEE_ID,
                tenant_id: TENANT_ID,
                name: None,
                description: None,
                fee_type: None,
                fee_structure: None,
                tax_category_id: Some(Some(nontaxable.id)),
            },
        )
        .await
        .map_err(test_error)?;
    let customer_id = customer(&setup, Some("FR")).await?;
    fund_promotional_grant(
        &setup.store,
        TENANT_ID,
        customer_id,
        None,
        Grant::invoice(1_000),
    )
    .await;
    let standard = crate::test_customer_credit::line("Standard", 1_000, false);
    let exempt = meteroid_store::domain::LineItem {
        product_id: Some(PRODUCT_PLATFORM_FEE_ID),
        ..crate::test_customer_credit::line("Exempt", 500, false)
    };
    let content = oneoff_content(&setup, customer_id, vec![standard, exempt]).await?;
    assert_eq!(content.promotional_credits, 1_000);
    let draft = draft(&setup, customer_id, &content, false).await?;
    fund_promotional_grant(
        &setup.store,
        TENANT_ID,
        customer_id,
        None,
        Grant::invoice(500),
    )
    .await;

    let finalized = setup
        .services
        .finalize_invoice(Actor::System, draft.id, TENANT_ID)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(finalized.promotional_credits, 1_500);
    assert_eq!((finalized.tax_amount, finalized.total), (0, 0));
    let standard_row = finalized
        .tax_breakdown
        .iter()
        .find(|row| row.tax_rate == dec!(0.2))
        .ok_or("the 20% row stays")?;
    assert_eq!(standard_row.exemption_type, None);
    let exempt_row = finalized
        .tax_breakdown
        .iter()
        .find(|row| row.tax_rate.is_zero())
        .ok_or("the exempt line keeps its row")?;
    assert_eq!(exempt_row.exemption_type, Some(TaxExemptionType::TaxExempt));
    let findings = vat_findings(&setup, finalized.id).await?;
    assert!(findings.is_empty(), "{findings:#?}");
    Ok(())
}
