//! Credit-pack plans shaped like the Halyard sandbox, built on the seeded plans: "Scale" (Pro)
//! sells a pack resetting each period and a welcome credit once, "Studio" (Starter) includes
//! credits each period.
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use common_domain::ids::PlanVersionId;
use meteroid_store::clients::usage::MutableMockUsageClient;
use meteroid_store::domain::credit_units::CreditUnitNew;
use meteroid_store::domain::enums::{BillingPeriodEnum, FeeTypeEnum};
use meteroid_store::domain::price_components::{
    CreditExpiry, CreditRollover, PriceEntry, PriceInput, ProductRef,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::repositories::CreditUnitsInterface;
use meteroid_store::repositories::price_components::PriceComponentInterface;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::{
    PLAN_VERSION_ENTERPRISE_ID, PLAN_VERSION_PRO_ID, PLAN_VERSION_STARTER_ID, TENANT_ID,
};
use crate::harness::{TestEnv, test_env_with_seed_and_usage};
use crate::meteroid_it::container::SeedLevel;

pub const SCALE: PlanVersionId = PLAN_VERSION_PRO_ID;
pub const STUDIO: PlanVersionId = PLAN_VERSION_STARTER_ID;
pub const LARGE: PlanVersionId = PLAN_VERSION_ENTERPRISE_ID;

pub fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

pub fn midnight(day: NaiveDate) -> DateTime<Utc> {
    day.and_time(NaiveTime::MIN).and_utc()
}

pub async fn add_component(
    env: &TestEnv,
    name: &str,
    plan_version_id: PlanVersionId,
    product: ProductRef,
    pricing: Pricing,
) {
    crate::helpers::plans::as_draft(
        env.store(),
        plan_version_id,
        env.store().create_price_component_from_ref(
            name.to_owned(),
            product,
            vec![PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_owned(),
                pricing,
                credit_unit: None,
            })],
            plan_version_id,
            TENANT_ID,
        ),
    )
    .await
    .unwrap_or_else(|e| panic!("{name}: {e:?}"));
}

pub fn new_pack_product(name: &str) -> ProductRef {
    ProductRef::New {
        name: name.to_owned(),
        fee_type: FeeTypeEnum::Credit,
        fee_structure: FeeStructure::Credit {},
    }
}

/// Scale: Platform Fee 99 + Seats 25 + a EUR 500 pack of EUR 550 (`scale_expiry`) + EUR 50 once.
/// Studio: Platform Fee 29 + Seats 10 + 5,000 included credits (precision 2) for the period.
pub async fn halyard(env: &TestEnv, scale_expiry: CreditExpiry) {
    let unit_id = env
        .store()
        .create_credit_unit(
            CreditUnitNew::new(TENANT_ID, "CREDITS", "Credit", "Credits", None, 2).unwrap(),
        )
        .await
        .expect("credit unit")
        .id;
    add_component(
        env,
        "Monthly credit",
        SCALE,
        new_pack_product("Monthly credit"),
        Pricing::Credit {
            unit_price: dec!(500),
            quantity: 1,
            credit_amount: dec!(550),
            expiry: Some(scale_expiry),
            credit_unit_id: None,
            rollover: None,
            once: false,
        },
    )
    .await;
    add_component(
        env,
        "Welcome credit",
        SCALE,
        new_pack_product("Welcome credit"),
        Pricing::Credit {
            unit_price: Decimal::ZERO,
            quantity: 1,
            credit_amount: dec!(50),
            expiry: None,
            credit_unit_id: None,
            rollover: None,
            once: true,
        },
    )
    .await;
    add_component(
        env,
        "Monthly credits",
        STUDIO,
        new_pack_product("Monthly credits"),
        Pricing::Credit {
            unit_price: Decimal::ZERO,
            quantity: 1,
            credit_amount: dec!(5000),
            expiry: Some(CreditExpiry::BillingPeriod),
            credit_unit_id: Some(unit_id),
            rollover: Some(CreditRollover {
                max_amount: Some(dec!(1000)),
                expiry: CreditExpiry::Month(1),
            }),
            once: false,
        },
    )
    .await;
}

/// The seeded plans over a usage client the test sets.
pub async fn usage_env() -> (TestEnv, MutableMockUsageClient) {
    let usage = MutableMockUsageClient::new();
    let env = test_env_with_seed_and_usage(SeedLevel::PLANS, Arc::new(usage.clone())).await;
    (env, usage)
}
