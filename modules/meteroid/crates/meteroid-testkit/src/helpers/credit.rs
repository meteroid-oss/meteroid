//! Customer credit reads and grants every credit test states its amounts through.
use chrono::{DateTime, Utc};
use common_domain::ids::{BaseId, CreditGrantId, CustomerId, ProductId, SubscriptionId, TenantId};
use meteroid_store::Store;
use meteroid_store::domain::credit::{
    CreditApplication, CreditCost, CreditWallet, GrantClassification, GrantScope,
};
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::customer_credit::DEFAULT_PRIORITY;
use meteroid_store::repositories::credit::customer_credit_wallets::{
    FundCustomerCreditWallet, FundedCustomerCreditWallet,
};
use meteroid_store::repositories::customers::CustomersInterfaceAuto;

#[derive(Clone, Copy)]
pub struct Grant {
    pub application: CreditApplication,
    pub amount_minor: i64,
    pub priority: i32,
    pub expires_at: Option<DateTime<Utc>>,
}

impl Grant {
    pub fn invoice(amount_minor: i64) -> Self {
        Self {
            application: CreditApplication::Invoice,
            amount_minor,
            priority: DEFAULT_PRIORITY,
            expires_at: None,
        }
    }
}

async fn currency_or_customers(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: Option<&str>,
) -> String {
    match currency {
        Some(currency) => currency.to_owned(),
        None => {
            store
                .find_customer_by_id(customer_id, tenant_id)
                .await
                .expect("customer for invoice credit")
                .currency
        }
    }
}

/// Grants paid credit by hand (paid outside Meteroid) and returns it.
pub async fn fund_grant(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: Option<&str>,
    grant: Grant,
) -> CreditGrantId {
    fund_classified(
        store,
        tenant_id,
        customer_id,
        currency,
        grant,
        GrantClassification::Paid,
        Vec::new(),
        None,
    )
    .await
}

/// Grants promotional credit by hand, which invoices draw before tax, and returns it.
pub async fn fund_promotional_grant(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: Option<&str>,
    grant: Grant,
) -> CreditGrantId {
    fund_classified(
        store,
        tenant_id,
        customer_id,
        currency,
        grant,
        GrantClassification::Promotional,
        Vec::new(),
        None,
    )
    .await
}

/// Grants paid credit that pays only for the subscription's invoices.
pub async fn fund_subscription_grant(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    subscription_id: SubscriptionId,
    grant: Grant,
) -> CreditGrantId {
    fund_classified(
        store,
        tenant_id,
        customer_id,
        None,
        grant,
        GrantClassification::Paid,
        Vec::new(),
        Some(subscription_id),
    )
    .await
}

/// Grants credit that pays only for `products`.
pub async fn fund_product_grant(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    grant: Grant,
    classification: GrantClassification,
    products: Vec<ProductId>,
) -> CreditGrantId {
    fund_classified(
        store,
        tenant_id,
        customer_id,
        None,
        grant,
        classification,
        products,
        None,
    )
    .await
}

async fn fund_classified(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: Option<&str>,
    grant: Grant,
    classification: GrantClassification,
    products: Vec<ProductId>,
    subscription_id: Option<SubscriptionId>,
) -> CreditGrantId {
    let currency = currency_or_customers(store, tenant_id, customer_id, currency).await;
    let wallet = store
        .create_customer_credit_wallet(tenant_id, customer_id, &currency)
        .await
        .expect("open customer credit wallet");
    let grant_id = store
        .fund_scoped_customer_credit_wallet(
            FundCustomerCreditWallet {
                tenant_id,
                customer_id,
                wallet_id: wallet.id,
                amount_minor: grant.amount_minor,
                // Classification orders money credit around tax; a credit unit wallet has no tax, and
                // only promotional unit credit needs no cost.
                classification: if wallet.credit_unit.is_some() {
                    GrantClassification::Promotional
                } else {
                    classification
                },
                priority: grant.priority,
                expires_at: grant.expires_at,
                idempotency_key: uuid::Uuid::now_v7(),
                reason: "Integration-test customer credit".to_owned(),
                reference: None,
                actor_id: None,
                cost: None,
            },
            GrantScope {
                subscription_id,
                application: grant.application,
                product_ids: products,
            },
        )
        .await
        .expect("fund customer credit")
        .grant_id;
    // Funded at the wall clock, it must still pay the 2024 periods the harness bills.
    effective_from_fixture_start(store, grant_id).await;
    grant_id
}

/// Grants invoice credit; `None` funds the customer's own currency.
pub async fn fund(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: Option<&str>,
    amount_minor: i64,
) -> CreditGrantId {
    fund_grant(
        store,
        tenant_id,
        customer_id,
        currency,
        Grant::invoice(amount_minor),
    )
    .await
}

pub async fn wallets(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
) -> Vec<CreditWallet> {
    store
        .customer_credit_wallets(tenant_id, customer_id)
        .await
        .expect("list customer credit wallets")
}

/// Credit spendable now in the customer's one wallet of a currency; `None` reads the customer's
/// own currency.
pub async fn available(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: Option<&str>,
) -> i64 {
    let currency = currency_or_customers(store, tenant_id, customer_id, currency).await;
    wallets(store, tenant_id, customer_id)
        .await
        .iter()
        .filter(|w| w.currency == currency.to_uppercase())
        .map(|w| w.available_minor)
        .sum()
}

/// What a grant holds now, plus what lapsed unspent at its expiry.
pub async fn grant_remaining(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    grant_id: CreditGrantId,
) -> i64 {
    wallets(store, tenant_id, customer_id)
        .await
        .iter()
        .flat_map(|wallet| wallet.grants.iter())
        .find(|g| g.id == grant_id)
        .map(|g| g.remaining_minor + g.expired_minor.unwrap_or_default())
        .expect("grant is in a customer credit wallet")
}

/// Moves a grant's term into the past, beyond the settlement grace of the seeded invoicing entity
/// (its 12-hour grace period). Grant terms are immutable, so triggers are bypassed for this write.
pub async fn expire_grant(store: &Store, grant_id: CreditGrantId) {
    expire_grant_hours_ago(store, grant_id, 13).await;
}

/// When a credit fixture moved into the past takes effect: the harness's first billed period,
/// so it may pay the usage of every period a test bills (usage is paid only by credit effective
/// before its line ends).
pub const FIXTURE_EFFECTIVE_AT: &str = "2024-01-01 00:00:00+00";

/// Moves a grant's expiry `hours` into the past, past the settlement grace; it takes effect at
/// [`FIXTURE_EFFECTIVE_AT`].
pub async fn expire_grant_hours_ago(store: &Store, grant_id: CreditGrantId, hours: i64) {
    use diesel_async::SimpleAsyncConnection;
    store
        .get_conn()
        .await
        .expect("billing connection")
        .batch_execute(&format!(
            "SET session_replication_role = replica;
             UPDATE customer_credit_grant
                SET effective_at = LEAST(effective_at, '{FIXTURE_EFFECTIVE_AT}'::timestamptz),
                    expires_at = now() - interval '{hours} hours' WHERE id = '{}';
             SET session_replication_role = origin;",
            grant_id.as_uuid()
        ))
        .await
        .expect("move a grant's expiry into the past");
}

/// [`FIXTURE_EFFECTIVE_AT`] as an instant.
pub fn fixture_start() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
        .expect("the fixtures' start")
        .with_timezone(&Utc)
}

/// Dates a live grant's start at [`FIXTURE_EFFECTIVE_AT`], as if added before the billed periods
/// (fixtures fund at the wall clock while the harness bills 2024).
pub async fn effective_from_fixture_start(store: &Store, grant_id: CreditGrantId) {
    use diesel_async::SimpleAsyncConnection;
    store
        .get_conn()
        .await
        .expect("billing connection")
        .batch_execute(&format!(
            "SET session_replication_role = replica;
             UPDATE customer_credit_grant
                SET effective_at = LEAST(effective_at, '{FIXTURE_EFFECTIVE_AT}'::timestamptz)
              WHERE id = '{}';
             SET session_replication_role = origin;",
            grant_id.as_uuid()
        ))
        .await
        .expect("date a grant from the fixtures' start");
}

/// The ledger's invariant: every grant's balance equals the sum of its statement lines, and a
/// grant never holds more than it was funded with. An unswept expiry appears as a synthesized
/// line leaving only what the sweep will roll over, which the wallet's balance counts too.
pub async fn assert_ledger_balances(store: &Store, tenant_id: TenantId, customer_id: CustomerId) {
    use meteroid_store::domain::credit::CreditGrantState;
    for wallet in wallets(store, tenant_id, customer_id).await {
        let (entries, total) = store
            .customer_credit_ledger(tenant_id, wallet.id, 1000, 0)
            .await
            .expect("read customer credit statement");
        assert_eq!(entries.len() as u64, total, "statement fits one page");
        let mut rolling = 0_i64;
        let mut spendable = 0_i64;
        for grant in &wallet.grants {
            let replayed: i64 = entries
                .iter()
                .filter(|e| e.grant_id == grant.id)
                .map(|e| e.amount_minor)
                .sum();
            let left = grant.remaining_minor + grant.expired_minor.unwrap_or_default();
            if let Some(expired) = grant.expired_minor {
                assert!(
                    (0..=expired).contains(&replayed),
                    "grant {} drifted from its ledger",
                    grant.id
                );
                rolling += replayed;
            } else {
                assert_eq!(
                    replayed, grant.remaining_minor,
                    "grant {} drifted from its ledger",
                    grant.id
                );
                if grant.state != CreditGrantState::Pending {
                    spendable += grant.remaining_minor;
                }
            }
            assert!(left >= 0 && left <= grant.amount_minor);
        }
        assert_eq!(
            wallet.available_minor,
            spendable + rolling,
            "wallet {} counts exactly what its expired grants will roll over",
            wallet.id
        );
    }
}

/// The subscription hand-authored drafts bill once they sell credit packs: a pack always belongs
/// to a subscription. A plan with no components, so it bills nothing of its own.
pub async fn pack_subscription(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    currency: &str,
    create: bool,
) -> Option<common_domain::ids::SubscriptionId> {
    use common_domain::ids::{PlanId, PlanVersionId, SubscriptionId};
    use diesel_async::AsyncConnection;
    use diesel_models::enums::{
        BillingPeriodEnum, PlanStatusEnum, PlanTypeEnum, SubscriptionActivationConditionEnum,
        SubscriptionStatusEnum,
    };
    use diesel_models::errors::DatabaseErrorContainer;
    use diesel_models::plan_versions::PlanVersionRowNew;
    use diesel_models::plans::{PlanRowNew, PlanRowPatch};
    use diesel_models::subscriptions::{SubscriptionRow, SubscriptionRowNew};

    let derived = |label: &str| uuid::Uuid::new_v5(&customer_id.as_uuid(), label.as_bytes());
    let subscription_id = SubscriptionId::from(derived("credit-pack-subscription"));
    let mut conn = store.get_conn().await.expect("billing connection");
    if SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, subscription_id)
        .await
        .is_ok()
    {
        return Some(subscription_id);
    }
    if !create {
        return None;
    }
    let (plan_id, plan_version_id) = (
        PlanId::from(derived("credit-pack-plan")),
        PlanVersionId::from(derived("credit-pack-plan-version")),
    );
    let today = Utc::now().date_naive();
    conn.transaction(async |tx| {
        PlanRowNew {
            id: plan_id,
            name: "Credit pack plan".to_owned(),
            description: None,
            tenant_id,
            product_family_id: crate::data::ids::PRODUCT_FAMILY_ID,
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
            tenant_id,
            period_start_day: None,
            net_terms: 0,
            currency: currency.to_owned(),
            billing_cycles: None,
            trialing_plan_id: None,
            trial_is_free: false,
            uses_product_pricing: true,
            tax_inclusive: false,
            credit_exhausted_rule: None,
            credit_exhausted_caps: serde_json::json!({}),
        }
        .insert(tx)
        .await?;
        PlanRowPatch {
            id: plan_id,
            tenant_id,
            name: None,
            description: None,
            active_version_id: Some(Some(plan_version_id)),
            draft_version_id: None,
            self_service_rank: None,
            issues_license: None,
        }
        .update(tx)
        .await?;
        SubscriptionRowNew {
            id: subscription_id,
            customer_id,
            billing_day_anchor: 1,
            tenant_id,
            start_date: today,
            plan_version_id,
            created_at: Utc::now().naive_utc(),
            net_terms: 0,
            invoice_memo: None,
            invoice_threshold: None,
            activated_at: Some(Utc::now().naive_utc()),
            currency: currency.to_owned(),
            mrr_cents: 0,
            period: BillingPeriodEnum::Monthly,
            pending_checkout: false,
            end_date: None,
            trial_duration: None,
            activation_condition: SubscriptionActivationConditionEnum::OnStart,
            billing_start_date: Some(today),
            cycle_index: Some(0),
            status: SubscriptionStatusEnum::Active,
            current_period_start: today,
            current_period_end: None,
            next_cycle_action: None,
            auto_advance_invoices: false,
            charge_automatically: false,
            purchase_order: None,
            quote_id: None,
            backdate_invoices: false,
            payment_methods_config: None,
            imported_at: None,
            custom_properties: serde_json::json!({}),
            tax_inclusive: false,
            tax_inclusive_rate: None,
            usage_filter_property: None,
            usage_filter_value: None,
            usage_filter_since: None,
        }
        .insert(tx)
        .await?;
        Ok::<_, DatabaseErrorContainer>(())
    })
    .await
    .expect("a subscription for credit packs");
    Some(subscription_id)
}

/// Sets on credit unit `unit_id` a product of the digital services category, taxed 10 % in
/// France by a custom rate of the default invoicing entity, and returns it.
pub async fn reduced_tax_unit_product(
    store: &Store,
    unit_id: common_domain::ids::CreditUnitId,
) -> ProductId {
    use crate::data::ids::{INVOICING_ENTITY_ID, PRODUCT_FAMILY_ID, TENANT_ID};
    use diesel_models::products::ProductRowNew;
    use meteroid_store::domain::accounting::{TaxRateNew, TaxRateRule};
    use meteroid_store::domain::credit_units::CreditUnitPatch;
    use meteroid_store::repositories::CreditUnitsInterface;
    use meteroid_store::repositories::accounting::AccountingInterface;
    use meteroid_store::repositories::tax_categories::TaxCategoryInterface;
    use std::str::FromStr;

    let reduced = store
        .list_tax_categories(TENANT_ID)
        .await
        .expect("categories")
        .into_iter()
        .find(|category| category.key == "digital_services")
        .expect("built-in category");
    store
        .insert_custom_tax(
            TENANT_ID,
            TaxRateNew {
                invoicing_entity_id: INVOICING_ENTITY_ID,
                name: "Reduced rate".to_string(),
                tax_code: "REDUCED_FR".to_string(),
                tax_category_id: Some(reduced.id),
                rules: vec![TaxRateRule {
                    country: Some(common_domain::country::CountryCode::from_str("FR").expect("FR")),
                    region: None,
                    rate: rust_decimal_macros::dec!(0.10),
                }],
            },
        )
        .await
        .expect("reduced rate");
    let product_id = ProductId::new();
    let mut conn = store.get_conn().await.expect("connection");
    ProductRowNew {
        id: product_id,
        name: "Credits".to_owned(),
        description: None,
        tenant_id: TENANT_ID,
        product_family_id: PRODUCT_FAMILY_ID,
        fee_type: diesel_models::enums::FeeTypeEnum::Credit,
        fee_structure: serde_json::to_value(
            &meteroid_store::domain::prices::FeeStructure::Credit {},
        )
        .expect("fee structure"),
        catalog: true,
        tax_category_id: Some(reduced.id),
    }
    .insert(&mut conn)
    .await
    .expect("the unit's tax product");
    drop(conn);
    store
        .update_credit_unit(
            CreditUnitPatch::new(TENANT_ID, unit_id, None, None, None, None)
                .expect("patch")
                .with_product_id(Some(product_id)),
        )
        .await
        .expect("the unit's product");
    product_id
}

/// Paid credit of `face` added by hand to a wallet in the customer's currency, stated to have
/// cost `cost`.
pub async fn add_paid(
    store: &Store,
    tenant_id: TenantId,
    customer_id: CustomerId,
    face: i64,
    cost: i64,
) -> meteroid_store::StoreResult<FundedCustomerCreditWallet> {
    let currency = store
        .find_customer_by_id(customer_id, tenant_id)
        .await?
        .currency;
    let wallet = store
        .create_customer_credit_wallet(tenant_id, customer_id, &currency)
        .await?;
    store
        .fund_customer_credit_wallet(FundCustomerCreditWallet {
            tenant_id,
            customer_id,
            wallet_id: wallet.id,
            amount_minor: face,
            classification: GrantClassification::Paid,
            priority: 100,
            expires_at: Some(Utc::now() + chrono::Duration::days(365)),
            idempotency_key: uuid::Uuid::now_v7(),
            reason: "Paid outside Meteroid".to_owned(),
            reference: None,
            actor_id: None,
            cost: Some(CreditCost::at_face(cost, &currency)),
        })
        .await
}
