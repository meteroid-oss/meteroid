//! S11. Pay as you go plus a "Fine-tuning" product: a promotional grant restricted to fine-tuning
//! pays only fine-tuning lines, a customer-wide top-up pays the rest, and the invoice shows both.
use common_domain::actor::Actor;
use common_domain::ids::ProductId;
use meteroid_store::domain::credit::{CreditApplication, GrantClassification, GrantScope};
use meteroid_store::domain::enums::{BillingMetricAggregateEnum, BillingPeriodEnum, FeeTypeEnum};
use meteroid_store::domain::price_components::{
    PriceEntry, PriceInput, ProductRef, UsagePricingModel,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing, UsageModel};
use meteroid_store::domain::{BillableMetricNew, UnitConversionRoundingEnum};
use meteroid_store::repositories::ProductFamilyInterface;
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet;
use meteroid_store::repositories::price_components::PriceComponentInterface;
use rust_decimal_macros::dec;

use super::kit::{Halyard, LITE, M, STANDARD, Tax, at, at_time, day, grant, lines, money};
use super::s10_two_subscriptions::top_up;

/// PAYG+FT: fine-tuning tokens on `halyard_training_tokens`, $8 per 1M, a product of its own.
async fn fine_tuning(h: &Halyard) -> ProductId {
    let family = h
        .store()
        .find_default_product_family(h.tenant)
        .await
        .expect("product family");
    let metric = h
        .store()
        .insert_billable_metric(
            Actor::System,
            BillableMetricNew {
                name: "Training tokens".to_owned(),
                description: None,
                code: "halyard_training_tokens".to_owned(),
                aggregation_type: BillingMetricAggregateEnum::Sum,
                aggregation_key: Some(TRAINING.to_owned()),
                unit_conversion_factor: Some(1_000_000),
                unit_conversion_rounding: Some(UnitConversionRoundingEnum::None),
                segmentation_matrix: None,
                filters: None,
                usage_group_key: None,
                unit_label: None,
                tenant_id: h.tenant,
                product_family_id: family.id,
                product_id: None,
            },
        )
        .await
        .expect("the training metric");
    crate::helpers::plans::as_draft(
        h.store(),
        h.payg,
        h.store().create_price_component_from_ref(
            "Fine-tuning".to_owned(),
            ProductRef::New {
                name: "Fine-tuning".to_owned(),
                fee_type: FeeTypeEnum::Usage,
                fee_structure: FeeStructure::Usage {
                    metric_id: metric.id,
                    model: UsageModel::PerUnit,
                },
            },
            vec![PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "USD".to_owned(),
                pricing: Pricing::Usage(UsagePricingModel::PerUnit { rate: dec!(8) }),
                credit_unit: None,
            })],
            h.payg,
            h.tenant,
        ),
    )
    .await
    .expect("fine-tuning on Pay as you go")
    .product_id
    .expect("the fine-tuning product")
}

const TRAINING: &str = "training_tokens";

#[tokio::test]
async fn s11_a_product_restricted_grant_pays_only_its_product() {
    let h = Halyard::new().await;
    let fine_tuning = fine_tuning(&h).await;
    let customer = h.customer("Untaxed", Tax::Untaxed).await;
    let (sub, _) = h.subscribe(customer, h.payg, day(9, 1)).await;

    let wallet = h
        .store()
        .create_customer_credit_wallet(h.tenant, customer, "USD")
        .await
        .expect("wallet");
    let restricted = h
        .store()
        .fund_scoped_customer_credit_wallet_at(
            FundCustomerCreditWallet {
                tenant_id: h.tenant,
                customer_id: customer,
                wallet_id: wallet.id,
                amount_minor: 3_000,
                classification: GrantClassification::Promotional,
                priority: DEFAULT_PRIORITY,
                expires_at: None,
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "Fine-tuning trial".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            GrantScope {
                subscription_id: None,
                application: CreditApplication::Invoice,
                product_ids: vec![fine_tuning],
            },
            at_time(9, 1, 10, 0, 0),
        )
        .await
        .expect("G-ft")
        .grant_id;
    h.check().await;

    // Sep 5: fine-tuning 2M (1600); Sep 6: top-up 5000; Sep 7-30: model usage standard 2.5M
    // (3000) + lite 5M (1000), fine-tuning 3M (2400).
    let training = |when, tokens: u64| {
        let tokens = tokens.to_string();
        h.usage
            .record_with(customer, when, &[(TRAINING, tokens.as_str())]);
    };
    training(at_time(9, 5, 10, 0, 0), 2 * M);
    let top_up = top_up(&h, customer, 5_000, at(9, 6)).await;
    h.tokens(customer, at_time(9, 10, 10, 0, 0), STANDARD, 5 * M / 2);
    h.tokens(customer, at_time(9, 12, 10, 0, 0), LITE, 5 * M);
    training(at_time(9, 20, 10, 0, 0), 3 * M);
    h.check().await;

    let invoice = h.renew(sub, at(10, 1)).await.expect("INV-1");
    let invoice = h.invoice(invoice.id).await;
    assert_eq!(
        invoice
            .line_items
            .iter()
            .filter(|line| line.product_id == Some(fine_tuning))
            .map(|line| line.amount_subtotal)
            .sum::<i64>(),
        4_000,
        "fine-tuning 5M × $8: {:?}",
        lines(&invoice)
    );
    assert_eq!(
        money(&invoice),
        (8_000, 0, 3_000, 0, 5_000, 5_000, 0),
        "G-ft before tax on fine-tuning only; the top-up pays fine-tuning's 1000 and the model's 4000"
    );
    let ft_line = invoice
        .line_items
        .iter()
        .find(|line| line.product_id == Some(fine_tuning))
        .expect("the fine-tuning line");
    assert_eq!(ft_line.promotional_credit, Some(3_000));
    assert!(
        invoice
            .line_items
            .iter()
            .filter(|line| line.product_id != Some(fine_tuning))
            .all(|line| line.promotional_credit.unwrap_or(0) == 0),
        "the restricted grant never pays model usage"
    );
    let grants = h.grants(customer).await;
    assert_eq!(
        grants
            .iter()
            .find(|g| g.id == restricted)
            .unwrap()
            .remaining,
        0
    );
    assert_eq!(grants.iter().find(|g| g.id == top_up).unwrap().remaining, 0);
    assert_eq!(
        grant(&grants, "MANUAL", "PROMOTIONAL", 3_000).id,
        restricted
    );
    h.check().await;
}
