//! Credit unit wallets funded by plan credit packs: a paid pack is billed in money and taxed when
//! sold (its credits are never redeemed as money), then funds the unit's wallet at its net cost;
//! an included pack grants its credits each period at no cost.
use std::str::FromStr;
use std::sync::Arc;

use chrono::Utc;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use common_domain::actor::Actor;
use common_domain::country::CountryCode;
use common_domain::ids::{BaseId, CreditUnitId, CustomerId, ProductId, SubscriptionId};
use meteroid::workers::pgmq::processors::run_once_invoice_orchestration;
use meteroid_store::clients::usage::MockUsageClient;
use meteroid_store::domain::credit::{
    CreditApplication, CreditCost, CreditGrantSource, CreditWallet, GrantClassification,
    GrantRollover,
};
use meteroid_store::domain::credit_units::CreditUnitNew;
use meteroid_store::domain::enums::{FeeTypeEnum, InvoiceStatusEnum, TaxResolverEnum};
use meteroid_store::domain::price_components::{
    CreditExpiry, CreditRollover, PriceEntry, PriceInput, ProductRef,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::{Address, Invoice, InvoicingEntityPatch, LineItem, LineItemKind};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CreditUnitsInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::price_components::PriceComponentInterface;

use super::usage::{PLAN_VERSION_USAGE_ONLY_ID, seed_usage_only_plan};
use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;

const UNIT_CODE: &str = "CREDITS";

pub(super) struct PackScenario {
    pub(super) env: TestEnv,
    pub(super) customer_id: CustomerId,
    pub(super) unit_id: CreditUnitId,
    subscription_id: SubscriptionId,
    unit_product: Option<ProductId>,
}

/// A unit of precision 2; the usage-only EUR plan gains a paid pack of 5,000 credits for EUR 40
/// and 500 credits included each period (at most 200 rolled over for a month), both through the
/// store's price validation. A French B2B customer pays 20 % VAT.
pub(super) async fn pack_scenario() -> PackScenario {
    pack_scenario_with(false).await
}

/// `pack_scenario`, the unit's tax product set to a 10 % category when `reduced_unit_product`.
async fn pack_scenario_with(reduced_unit_product: bool) -> PackScenario {
    let env =
        test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(MockUsageClient::noop())).await;
    seed_usage_only_plan(&env).await;
    env.set_consolidate_and_grace(false, 0).await;
    let unit_id = env
        .store()
        .create_credit_unit(
            CreditUnitNew::new(TENANT_ID, UNIT_CODE, "Credit", "Credits", None, 2).unwrap(),
        )
        .await
        .expect("credit unit")
        .id;
    add_pack(
        &env,
        "Credit pack",
        Pricing::Credit {
            unit_price: dec!(40),
            quantity: 1,
            credit_amount: dec!(5000),
            expiry: None,
            credit_unit_id: Some(unit_id),
            rollover: None,
            once: false,
        },
    )
    .await
    .expect("a paid pack in credits");
    add_pack(
        &env,
        "Included credits",
        Pricing::Credit {
            unit_price: Decimal::ZERO,
            quantity: 1,
            credit_amount: dec!(500),
            expiry: Some(CreditExpiry::BillingPeriod),
            credit_unit_id: Some(unit_id),
            rollover: Some(CreditRollover {
                max_amount: Some(dec!(200)),
                expiry: CreditExpiry::Month(1),
            }),
            once: false,
        },
    )
    .await
    .expect("included credits");
    let unit_product = if reduced_unit_product {
        Some(crate::helpers::credit::reduced_tax_unit_product(env.store(), unit_id).await)
    } else {
        None
    };

    let customer_id = CustomerId::new();
    create_french_b2b_customer(&env, customer_id).await;
    env.store()
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                tax_resolver: Some(TaxResolverEnum::MeteroidEuVat),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("EU VAT on the invoicing entity");
    let subscription_id = subscription()
        .customer(customer_id)
        .plan_version(PLAN_VERSION_USAGE_ONLY_ID)
        .start_date(Utc::now().date_naive())
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    PackScenario {
        env,
        customer_id,
        unit_id,
        subscription_id,
        unit_product,
    }
}

async fn add_pack(
    env: &TestEnv,
    name: &str,
    pricing: Pricing,
) -> Result<(), error_stack::Report<StoreError>> {
    crate::helpers::plans::as_draft(
        env.store(),
        PLAN_VERSION_USAGE_ONLY_ID,
        env.store().create_price_component_from_ref(
            name.to_owned(),
            ProductRef::New {
                name: name.to_owned(),
                fee_type: FeeTypeEnum::Credit,
                fee_structure: FeeStructure::Credit {},
            },
            vec![PriceEntry::New(PriceInput {
                cadence: meteroid_store::domain::enums::BillingPeriodEnum::Monthly,
                currency: "EUR".to_owned(),
                pricing,
                credit_unit: None,
            })],
            PLAN_VERSION_USAGE_ONLY_ID,
            TENANT_ID,
        ),
    )
    .await
    .map(|_| ())
}

async fn create_french_b2b_customer(env: &TestEnv, customer_id: CustomerId) {
    use diesel_models::customers::CustomerRowNew;
    let address = Address {
        line1: Some("1 Rue de Rivoli".to_string()),
        line2: None,
        city: Some("Paris".to_string()),
        country: Some(CountryCode::from_str("FR").expect("country code")),
        state: None,
        zip_code: Some("75001".to_string()),
    };
    let mut conn = env.conn().await;
    CustomerRowNew {
        id: customer_id,
        name: "Unit Wallets SAS".to_string(),
        created_at: Some(Utc::now().naive_utc()),
        tenant_id: TENANT_ID,
        alias: None,
        currency: "EUR".to_string(),
        invoicing_entity_id: INVOICING_ENTITY_ID,
        billing_address: Some(serde_json::to_value(&address).unwrap()),
        shipping_address: None,
        billing_email: None,
        current_payment_method_id: None,
        vat_number: Some("FR12345678901".to_string()),
        legal_number: None,
        buyer_reference: None,
        custom_taxes: serde_json::json!([]),
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
    .expect("French customer");
}

impl PackScenario {
    async fn issued_invoice(&self) -> Invoice {
        self.env.process_due_events().await;
        let invoices = self.env.get_invoices(self.subscription_id).await;
        assert_eq!(invoices.len(), 1, "the period's advance invoice");
        let invoice = invoices.into_iter().next().unwrap();
        assert_eq!(invoice.status, InvoiceStatusEnum::Finalized);
        invoice
    }

    async fn pay(&self, invoice: &Invoice) {
        self.env
            .services()
            .mark_invoice_as_paid(
                Actor::System,
                TENANT_ID,
                invoice.id,
                Decimal::new(invoice.amount_due, 2),
                Utc::now().fixed_offset(),
                Some(format!("payment-{}", invoice.id)),
            )
            .await
            .expect("the invoice is paid");
        self.orchestrate().await;
    }

    async fn orchestrate(&self) {
        run_once_invoice_orchestration(
            Arc::new(self.env.store().clone()),
            Arc::new(self.env.services().clone()),
        )
        .await;
    }

    async fn wallets(&self) -> Vec<CreditWallet> {
        self.env
            .store()
            .customer_credit_wallets(TENANT_ID, self.customer_id)
            .await
            .expect("wallets")
    }

    /// The unit's wallet; no money wallet of the customer holds any credit.
    async fn unit_wallet(&self) -> CreditWallet {
        let (units, money): (Vec<_>, Vec<_>) = self
            .wallets()
            .await
            .into_iter()
            .partition(|wallet| wallet.currency == UNIT_CODE);
        assert!(
            money
                .iter()
                .all(|wallet| wallet.available_minor == 0 && wallet.grants.is_empty()),
            "a unit pack funds no money credit: {money:?}"
        );
        assert_eq!(units.len(), 1);
        let wallet = units.into_iter().next().unwrap();
        let unit = wallet.credit_unit.as_ref().expect("a credit unit wallet");
        assert_eq!((unit.id, unit.precision), (self.unit_id, 2));
        wallet
    }
}

fn line<'a>(invoice: &'a Invoice, name: &str) -> &'a LineItem {
    invoice
        .line_items
        .iter()
        .find(|line| line.name.starts_with(name))
        .unwrap_or_else(|| panic!("a {name} line"))
}

/// The pack is billed EUR 40 plus 20 % VAT; once paid it funds 5,000.00 credits (500,000 minor
/// units) costing the EUR 40 net, and no money credit: nothing of it is a prepayment.
#[tokio::test]
async fn a_unit_pack_is_taxed_when_sold_and_funds_its_credits_at_the_net_price() {
    let s = pack_scenario().await;
    let invoice = s.issued_invoice().await;

    let pack = line(&invoice, "Credit pack");
    assert_eq!(pack.kind, LineItemKind::Credit);
    assert_eq!(pack.amount_subtotal, 4_000);
    assert_eq!(
        pack.credit_grant_amount,
        Some(500_000),
        "5,000 credits at precision 2"
    );
    let granted = pack
        .credit_grant_unit
        .as_ref()
        .expect("granted in the unit");
    assert_eq!(
        (granted.unit_id, granted.code.as_str(), granted.precision),
        (s.unit_id, UNIT_CODE, 2)
    );
    assert!(pack.taxed_at_purchase);
    assert_eq!(pack.tax_rate, dec!(0.2));
    assert_eq!(pack.tax_amount, 800, "taxed when sold");
    assert_eq!(pack.amount_total, 4_800);
    assert_eq!(
        (invoice.subtotal, invoice.tax_amount, invoice.total),
        (4_000, 800, 4_800)
    );
    assert_eq!(invoice.applied_credits, 0, "credit never buys credit");
    assert!(
        !s.unit_wallet()
            .await
            .grants
            .iter()
            .any(|grant| grant.source == CreditGrantSource::Purchase),
        "an unpaid pack funds nothing"
    );

    s.pay(&invoice).await;
    let wallet = s.unit_wallet().await;
    let purchased: Vec<_> = wallet
        .grants
        .iter()
        .filter(|grant| grant.source == CreditGrantSource::Purchase)
        .collect();
    assert_eq!(purchased.len(), 1);
    let grant = purchased[0];
    assert_eq!(grant.amount_minor, 500_000);
    assert_eq!(grant.classification, GrantClassification::Paid);
    assert_eq!(grant.invoice_id, Some(invoice.id));
    assert_eq!(
        grant.cost,
        Some(CreditCost::at_face(4_000, "EUR")),
        "the net price, without the tax"
    );
    assert_eq!(
        grant.cost.as_ref().unwrap().per_unit(grant.amount_minor, 2),
        Some(dec!(0.008)),
        "EUR 0.008 per credit"
    );
    assert_eq!(
        wallet.available_minor, 550_000,
        "purchased and included credits"
    );

    s.orchestrate().await;
    assert_eq!(
        s.unit_wallet().await.available_minor,
        550_000,
        "a pack funds once"
    );
    crate::helpers::credit::assert_ledger_balances(s.env.store(), TENANT_ID, s.customer_id).await;
}

/// A plan's pack is taxed and booked under the unit's tax product, as the unit's direct sales
/// are, not under its component's: 10 % on EUR 40.
#[tokio::test]
async fn a_unit_pack_is_taxed_with_the_unit_s_tax_product() {
    let s = pack_scenario_with(true).await;
    let invoice = s.issued_invoice().await;

    let pack = line(&invoice, "Credit pack");
    assert!(s.unit_product.is_some());
    assert_eq!(pack.product_id, s.unit_product);
    assert!(pack.taxed_at_purchase);
    assert_eq!(
        (pack.tax_rate, pack.tax_amount, pack.amount_total),
        (dec!(0.10), 400, 4_400)
    );
    let included = line(&invoice, "Included credits");
    assert_eq!(included.product_id, s.unit_product);
    assert_eq!(included.tax_amount, 0);
    assert_eq!(
        (invoice.subtotal, invoice.tax_amount, invoice.total),
        (4_000, 400, 4_400)
    );
}

/// The included pack bills nothing and bears no tax; issuing the invoice grants its 500 credits
/// for the period as given credit costing nothing, with its rollover cap frozen in the unit's
/// minor units.
#[tokio::test]
async fn included_credit_in_a_unit_is_granted_each_period_at_no_cost() {
    let s = pack_scenario().await;
    let invoice = s.issued_invoice().await;

    let included = line(&invoice, "Included credits");
    assert!(included.credit_grant_included);
    assert_eq!(included.amount_subtotal, 0);
    assert_eq!(included.tax_amount, 0);
    assert_eq!(included.credit_grant_amount, Some(50_000));
    assert_eq!(
        included.credit_grant_rollover,
        Some(GrantRollover {
            max_minor: Some(20_000),
            expiry: CreditExpiry::Month(1),
        }),
        "the cap of 200 credits, in the unit's minor units"
    );

    let wallet = s.unit_wallet().await;
    let grants: Vec<_> = wallet
        .grants
        .iter()
        .filter(|grant| grant.source == CreditGrantSource::Included)
        .collect();
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].amount_minor, 50_000);
    assert_eq!(grants[0].classification, GrantClassification::Promotional);
    assert_eq!(grants[0].cost, Some(CreditCost::given("EUR")));
    assert_eq!(
        grants[0].expires_at,
        Some(included.end_date.and_time(chrono::NaiveTime::MIN).and_utc()),
        "included credit lapses with its period"
    );

    s.orchestrate().await;
    let again = s.unit_wallet().await;
    assert_eq!(
        again
            .grants
            .iter()
            .filter(|grant| grant.source == CreditGrantSource::Included)
            .count(),
        1,
        "the period's credit is granted once"
    );
    crate::helpers::credit::assert_ledger_balances(s.env.store(), TENANT_ID, s.customer_id).await;
}

/// A pack is refused when it grants finer than its unit, or grants an archived unit.
#[tokio::test]
async fn a_pack_grants_whole_minor_units_of_an_active_unit() {
    let s = pack_scenario().await;
    let pack = |credit_amount: Decimal, unit: CreditUnitId| Pricing::Credit {
        unit_price: dec!(10),
        quantity: 1,
        credit_amount,
        expiry: None,
        credit_unit_id: Some(unit),
        rollover: None,
        once: false,
    };
    let refused = add_pack(&s.env, "Too fine", pack(dec!(0.001), s.unit_id))
        .await
        .expect_err("0.001 credit is finer than precision 2");
    assert!(
        matches!(refused.current_context(), StoreError::InvalidArgument(_)),
        "{refused:?}"
    );

    let unit = s
        .env
        .store()
        .create_credit_unit(
            CreditUnitNew::new(TENANT_ID, "TOKENS", "Token", "Tokens", None, 0).unwrap(),
        )
        .await
        .expect("a second unit");
    assert!(
        add_pack(&s.env, "Tokens", pack(dec!(1000), unit.id))
            .await
            .is_ok()
    );
    s.env
        .store()
        .update_credit_unit(
            meteroid_store::domain::credit_units::CreditUnitPatch::new(
                TENANT_ID,
                unit.id,
                None,
                None,
                None,
                Some(true),
            )
            .unwrap(),
        )
        .await
        .expect("archived");
    let archived = add_pack(&s.env, "Archived tokens", pack(dec!(1000), unit.id))
        .await
        .expect_err("an archived unit sells no new pack");
    assert!(
        matches!(
            archived.current_context(),
            StoreError::FailedPrecondition(_)
        ),
        "{archived:?}"
    );
}

/// Credit in a unit only pays the usage priced in that unit: a grant of it cannot be limited to
/// usage.
#[tokio::test]
async fn a_unit_wallet_pays_all_its_unit_s_usage() {
    let s = pack_scenario().await;
    let wallet = s
        .env
        .store()
        .create_customer_credit_wallet(TENANT_ID, s.customer_id, UNIT_CODE)
        .await
        .expect("a unit wallet");
    let refused = s
        .env
        .store()
        .fund_scoped_customer_credit_wallet(
            meteroid_store::repositories::credit::customer_credit_wallets::FundCustomerCreditWallet {
                tenant_id: TENANT_ID,
                customer_id: s.customer_id,
                wallet_id: wallet.id,
                amount_minor: 1_000,
                classification: GrantClassification::Promotional,
                priority: 100,
                expires_at: None,
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "Usage-only credits".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            meteroid_store::domain::credit::GrantScope {
                subscription_id: None,
                application: CreditApplication::Usage,
                product_ids: Vec::new(),
            },
        )
        .await;
    assert!(
        format!("{refused:?}").contains("cannot be limited to usage"),
        "{refused:?}"
    );
    let unknown = s
        .env
        .store()
        .create_customer_credit_wallet(TENANT_ID, s.customer_id, "TOKENS")
        .await;
    assert!(
        unknown.is_err(),
        "an unknown unit opens no wallet: {unknown:?}"
    );
    let opened = s
        .env
        .store()
        .create_customer_credit_wallet(TENANT_ID, s.customer_id, UNIT_CODE)
        .await
        .expect("a unit wallet");
    assert_eq!(
        opened.credit_unit.map(|unit| unit.code.into_inner()),
        Some(UNIT_CODE.to_owned())
    );
}
