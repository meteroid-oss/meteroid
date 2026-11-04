//! Seeding of the prepaid-credit sandbox: the credit units, the mock gateway and the bank account
//! invoices are paid to, and the usage history the replay bills.

use std::collections::HashMap;

use chrono::NaiveDateTime;
use common_domain::actor::Actor;
use common_domain::country::CountryCode;
use common_domain::ids::{
    BankAccountId, BaseId, ConnectorId, CreditUnitId, CustomerId, ProductFamilyId, TenantId,
};
use meteroid_store::domain::connectors::{MockPublicData, PaymentCredentialsUpdate};
use meteroid_store::domain::coupons::{CouponDiscount, CouponNew};
use meteroid_store::domain::credit_units::{CreditUnit, CreditUnitNew, CreditUnitPatch};
use meteroid_store::domain::enums::BankAccountFormat;
use meteroid_store::domain::enums::FeeTypeEnum;
use meteroid_store::domain::price_components::MatrixRow;
use meteroid_store::domain::prices::FeeStructure;
use meteroid_store::domain::products::ProductNew;
use meteroid_store::domain::{
    BankAccountNew, BillableMetric, InvoicingEntity, InvoicingEntityPatch,
    InvoicingEntityProvidersPatch, Tenant, UsagePricingModel,
};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::CreditUnitsInterface;
use meteroid_store::repositories::bank_accounts::BankAccountsInterface;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::products::ProductInterface;
use meteroid_store::services::sandbox_replay::SandboxReplayInterface;
use meteroid_store::{Services, Store, StoreResult};
use rust_decimal::Decimal;

use crate::presets::credit_traffic::{MODEL_PROPERTY, MODELS};
use crate::presets::history_usage::{self, CustomerUsage, ModelPrices};
use crate::presets::scenarios::domain::{
    Collection, CreditUnitSpec, Customer, FeeType, Plan, PromoCodeSpec,
};
use crate::presets::scenarios::timeline::{Action, Clock};
use crate::usage;

pub(super) const GATEWAY_ALIAS: &str = "sandbox-gateway";

pub(super) async fn create_credit_units(
    store: &Store,
    tenant_id: TenantId,
    specs: &[CreditUnitSpec],
) -> StoreResult<HashMap<String, CreditUnit>> {
    let mut units = HashMap::new();
    for spec in specs {
        let unit = store
            .create_credit_unit(CreditUnitNew::new(
                tenant_id,
                spec.code,
                spec.name,
                spec.plural_name,
                spec.symbol,
                spec.precision,
            )?)
            .await?;
        units.insert(spec.code.to_string(), unit);
    }
    Ok(units)
}

/// Gives every unit with a selling price the product its sales are booked (and taxed) under, then
/// its prices: the replay sells units at their price as it stands.
pub(super) async fn sell_credit_units(
    store: &Store,
    tenant_id: TenantId,
    product_family_id: ProductFamilyId,
    specs: &[CreditUnitSpec],
    units: &mut HashMap<String, CreditUnit>,
) -> StoreResult<()> {
    for spec in specs.iter().filter(|spec| !spec.selling_prices.is_empty()) {
        let unit = units.get(spec.code).ok_or_else(|| {
            StoreError::ValueNotFound(format!("Credit unit was not found {}", spec.code))
        })?;
        let product = store
            .create_product(
                Actor::System,
                ProductNew {
                    name: spec.plural_name.to_string(),
                    description: None,
                    tenant_id,
                    family_id: product_family_id,
                    fee_type: FeeTypeEnum::Credit,
                    fee_structure: FeeStructure::Credit {},
                    catalog: true,
                    tax_category_id: None,
                },
            )
            .await?;
        let unit = store
            .update_credit_unit(
                CreditUnitPatch::new(tenant_id, unit.id, None, None, None, None)?
                    .with_product_id(Some(product.id)),
            )
            .await?;
        for (currency, price) in &spec.selling_prices {
            store
                .set_credit_unit_price(tenant_id, unit.id, currency, *price)
                .await?;
        }
        units.insert(spec.code.to_string(), unit);
    }
    Ok(())
}

/// The promotion codes customers redeem for promotional credit, in the catalog since `at`.
pub(super) async fn create_promo_codes(
    services: &Services,
    tenant_id: TenantId,
    specs: &[PromoCodeSpec],
    at: NaiveDateTime,
) -> StoreResult<()> {
    for spec in specs {
        services
            .create_coupon_at(
                Actor::System,
                CouponNew {
                    code: spec.code.to_string(),
                    description: spec.name.to_string(),
                    tenant_id,
                    discount: CouponDiscount::Credit {
                        currency: spec.currency.to_string(),
                        amount: spec.amount,
                        expires_in_days: Some(spec.expires_after_days),
                        scope: Default::default(),
                    },
                    expires_at: None,
                    redemption_limit: None,
                    recurring_value: None,
                    reusable: false,
                    plan_ids: vec![],
                },
                at.and_utc(),
            )
            .await?;
    }
    Ok(())
}

pub(super) fn unit_ids(units: &HashMap<String, CreditUnit>) -> HashMap<String, CreditUnitId> {
    units
        .iter()
        .map(|(code, unit)| (code.clone(), unit.id))
        .collect()
}

/// The mock gateway, where saved cards are charged; invoices are not offered for card payment.
pub(super) async fn connect_mock_provider(
    store: &Store,
    tenant_id: TenantId,
) -> StoreResult<ConnectorId> {
    let connector = store
        .connect_payment_provider(
            Actor::System,
            tenant_id,
            GATEWAY_ALIAS.to_string(),
            PaymentCredentialsUpdate::Mock(MockPublicData::default()),
        )
        .await?;
    Ok(connector.id)
}

/// Connects the mock gateway as the card provider and opens the customer portal's top-up flows.
pub(super) async fn connect_gateway(
    store: &Store,
    tenant_id: TenantId,
    invoicing_entity: &InvoicingEntity,
) -> StoreResult<ConnectorId> {
    let connector_id = connect_mock_provider(store, tenant_id).await?;

    store
        .patch_invoicing_entity_providers(
            Actor::System,
            InvoicingEntityProvidersPatch {
                id: invoicing_entity.id,
                card_provider_id: Some(Some(connector_id)),
                direct_debit_provider_id: None,
                bank_account_id: None,
            },
            tenant_id,
        )
        .await?;

    store
        .patch_invoicing_entity(
            Actor::System,
            InvoicingEntityPatch {
                id: invoicing_entity.id,
                portal_credit_top_up_enabled: Some(true),
                portal_auto_top_up_enabled: Some(true),
                portal_budgets_enabled: Some(true),
                ..Default::default()
            },
            tenant_id,
        )
        .await?;

    Ok(connector_id)
}

/// The account invoices sent for bank transfer are paid to, when a plan collects that way.
pub(super) async fn connect_bank_account(
    store: &Store,
    tenant_id: TenantId,
    invoicing_entity: &InvoicingEntity,
    plans: &[Plan],
) -> StoreResult<Option<BankAccountId>> {
    let currencies: Vec<&str> = plans
        .iter()
        .filter(|p| {
            p.payment.collection
                == Collection::SendInvoice {
                    bank_transfer: true,
                }
        })
        .map(|p| p.currency.as_str())
        .collect();
    let Some(currency) = currencies.first() else {
        return Ok(None);
    };
    let account = store
        .insert_bank_account(
            Actor::System,
            BankAccountNew {
                id: BankAccountId::new(),
                tenant_id,
                currency: currency.to_string(),
                country: CountryCode::parse_as_opt("US")
                    .ok_or_else(|| StoreError::InvalidArgument("US is a country".to_string()))?,
                bank_name: "First Harbor Bank".to_string(),
                format: BankAccountFormat::AccountRouting,
                account_numbers: "000123456789 121000248".to_string(),
            },
        )
        .await?;
    store
        .patch_invoicing_entity_providers(
            Actor::System,
            InvoicingEntityProvidersPatch {
                id: invoicing_entity.id,
                card_provider_id: None,
                direct_debit_provider_id: None,
                bank_account_id: Some(Some(account.id)),
            },
            tenant_id,
        )
        .await?;
    Ok(Some(account.id))
}

/// List prices of the plan's model usage, in the denomination it is priced in.
pub(super) fn model_prices(plan: &Plan, units: &[CreditUnitSpec]) -> Option<ModelPrices> {
    plan.components.iter().find_map(|component| {
        let FeeType::Usage {
            pricing: UsagePricingModel::Matrix { rates },
            credit_unit,
            ..
        } = &component.fee
        else {
            return None;
        };
        let rate = |model: &str| -> Option<Decimal> {
            rates
                .iter()
                .find(|r: &&MatrixRow| {
                    r.dimension1.key == MODEL_PROPERTY && r.dimension1.value == model
                })
                .map(|r| r.per_unit_price)
        };
        let scale = match credit_unit {
            Some(unit) => u32::from(units.iter().find(|u| u.code == unit.credit_unit)?.precision),
            None => meteroid_money::iso::find(&plan.currency)?.exponent as u32,
        };
        Some(ModelPrices {
            per_million: [rate(MODELS[0])?, rate(MODELS[1])?, rate(MODELS[2])?],
            scale,
        })
    })
}

/// The plan a history subscribes to.
pub(super) fn subscribed_plan<'a>(customer: &Customer, plans: &'a [Plan]) -> Option<&'a Plan> {
    let history = customer.history.as_ref()?;
    history.events.iter().find_map(|event| match &event.action {
        Action::Subscribe { plan, .. } => plans.iter().find(|p| p.name == *plan),
        _ => None,
    })
}

/// One credit customer's generated usage and the window it covers.
pub(super) struct HistoryUsage {
    pub customer_id: CustomerId,
    pub clock: Clock,
    pub usage: CustomerUsage,
}

/// Generates every credit customer's usage history; refuses a target it cannot hit exactly.
pub(super) fn history_usage(
    customers: &[Customer],
    plans: &[Plan],
    units: &[CreditUnitSpec],
    customer_map: &HashMap<String, CustomerId>,
    as_of: NaiveDateTime,
) -> StoreResult<Vec<HistoryUsage>> {
    let mut out = Vec::new();
    for customer in customers {
        let (Some(history), Some(customer_id)) =
            (&customer.history, customer_map.get(&customer.name).copied())
        else {
            continue;
        };
        let clock = Clock::new(as_of, history.cycle_age_days);
        let prices = subscribed_plan(customer, plans).and_then(|p| model_prices(p, units));
        let usage = history_usage::generate(customer_id, &history.usage, &clock, prices.as_ref())
            .map_err(|e| {
            StoreError::InvalidArgument(format!("usage history of {}: {e}", customer.name))
        })?;
        out.push(HistoryUsage {
            customer_id,
            clock,
            usage,
        });
    }
    Ok(out)
}

/// Ingests the generated history and waits until every customer's total is queryable: billing a
/// cycle before its usage is materialized would invoice it short.
pub(super) async fn ingest_history_usage(
    services: &Services,
    tenant: &Tenant,
    metric: &BillableMetric,
    histories: &[HistoryUsage],
) -> StoreResult<()> {
    let tenant_id = tenant.id;
    let events: Vec<_> = histories
        .iter()
        .flat_map(|h| h.usage.events.iter().cloned().map(usage::to_ingest_event))
        .collect();
    log::info!(
        "sandbox credit history: {} events for tenant {tenant_id}",
        events.len()
    );
    let client = services.usage_clients();
    usage::ingest_all(client.clone(), tenant_id, tenant.keep_last_events, events).await;

    let expected: Vec<super::runner::ExpectedUsage> = histories
        .iter()
        .filter_map(|h| {
            let from = h.usage.first_event_at()?;
            Some(super::runner::ExpectedUsage {
                customer_id: h.customer_id,
                metric: metric.clone(),
                period: meteroid_store::domain::UsagePeriod {
                    start: from,
                    end: h.clock.as_of,
                },
                total: Decimal::from(h.usage.total_tokens()),
            })
        })
        .collect();
    super::runner::await_usage_materialized(
        client.as_ref(),
        tenant_id,
        &expected,
        super::usage_wait::CREDIT_USAGE_WAIT,
    )
    .await
    .map_err(|missing| {
        StoreError::FailedPrecondition(format!(
            "sandbox credit history is not queryable for tenant {tenant_id}: {}",
            missing.join("; ")
        ))
        .into()
    })
}
