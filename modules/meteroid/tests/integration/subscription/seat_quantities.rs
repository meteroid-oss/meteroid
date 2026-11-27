//! Seats count a managed quantity, named by the charge (R-137): seat snapshots stored before
//! quantities were named still count the quantity coded as their unit (I32); two charges with
//! the same unit label count two quantities; a plan charge and an add-on naming one quantity share
//! its count, the add-on billing that count on one instance (R-138); a product charged as a flat
//! fee in one plan and per seat in another is two charges (R-139). A removal waits for the
//! earliest period end of the charges counting its quantity, the bounds holding at each one's
//! renewal (R-108); a seat add-on is held once (R-138); a unit names one quantity whatever its
//! spelling (R-137).
use chrono::{NaiveDate, NaiveTime};
use common_domain::actor::Actor;
use common_domain::ids::{
    AddOnId, BaseId, ManagedQuantityId, PlanId, PlanVersionId, PriceComponentId, PriceId,
    ProductId, SubscriptionId,
};
use diesel_models::enums::BillingPeriodEnum as DieselBillingPeriodEnum;
use diesel_models::managed_quantities::{ManagedQuantityRow, ManagedQuantityRowNew};
use meteroid_store::domain::add_ons::AddOnNew;
use meteroid_store::domain::enums::{FeeTypeEnum, InvoiceType};
use meteroid_store::domain::price_components::{
    DowngradePolicy, PriceEntry, PriceInput, ProductRef, RatingModel, UpgradePolicy,
    UsagePricingModel,
};
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::subscription_add_ons::{
    CreateSubscriptionAddOn, CreateSubscriptionAddOns, SubscriptionAddOnCustomization,
};
use meteroid_store::domain::subscription_amendment::{
    AddOnChanges, ComponentChanges, SubscriptionAmendment,
};
use meteroid_store::domain::subscription_changes::PlanChangeMode;
use meteroid_store::domain::subscription_components::{
    ComponentParameterization, ComponentParameters,
};
use meteroid_store::domain::{BillingPeriodEnum, Invoice, SlotUpgradeBillingMode};
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::subscriptions::SubscriptionInterfaceAuto;
use meteroid_store::repositories::subscriptions::slots::SubscriptionSlotsInterfaceAuto;
use rstest::rstest;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{TestEnv, subscription, test_env};

fn day(m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, m, d).unwrap()
}

fn seats_counting(quantity: ManagedQuantityId) -> FeeStructure {
    FeeStructure::Slot {
        unit_name: "Seats".to_string(),
        quantity_id: Some(quantity),
        upgrade_policy: UpgradePolicy::Prorated,
        downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
    }
}

/// A new quantity of the tenant coded `code`.
async fn quantity(env: &TestEnv, code: &str) -> ManagedQuantityId {
    let id = ManagedQuantityId::new();
    ManagedQuantityRow::insert(
        &mut env.conn().await,
        &ManagedQuantityRowNew {
            id,
            tenant_id: TENANT_ID,
            code: code.to_string(),
            name: code.to_string(),
        },
    )
    .await
    .expect("a quantity")
    .expect("a new code");
    id
}

/// A product of the catalog's family, with no structure: its charges own one.
async fn product(env: &TestEnv, name: &str) -> ProductId {
    use diesel_async::RunQueryDsl;
    let id = ProductId::new();
    diesel::insert_into(diesel_models::schema::product::table)
        .values(&diesel_models::products::ProductRowNew {
            id,
            name: name.to_string(),
            description: None,
            tenant_id: TENANT_ID,
            product_family_id: PRODUCT_FAMILY_ID,
            catalog: true,
            tax_category_id: None,
            icon: None,
            color: None,
        })
        .execute(&mut env.conn().await)
        .await
        .expect("a product");
    id
}

pub(super) async fn on_starter(env: &TestEnv) -> SubscriptionId {
    subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await
}

pub(super) async fn change_seats(
    env: &TestEnv,
    sub_id: SubscriptionId,
    charge: impl Into<meteroid_store::domain::SeatCharge>,
    delta: i32,
    on: NaiveDate,
) {
    env.services()
        .update_subscription_slots_for_test(
            TENANT_ID,
            sub_id,
            charge,
            delta,
            SlotUpgradeBillingMode::Optimistic,
            Some(on.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("change seats");
}

async fn seats_of(
    env: &TestEnv,
    sub_id: SubscriptionId,
    quantity: ManagedQuantityId,
    at: NaiveDate,
) -> u32 {
    env.store()
        .get_active_slots_value(
            TENANT_ID,
            sub_id,
            quantity,
            Some(at.and_time(NaiveTime::MIN)),
        )
        .await
        .expect("seats")
}

pub(super) async fn renewal_on(env: &TestEnv, sub_id: SubscriptionId, on: NaiveDate) -> Invoice {
    env.get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| {
            invoice.invoice_type == InvoiceType::Recurring && invoice.billing_period_start == on
        })
        .unwrap_or_else(|| panic!("the renewal of {on}"))
}

pub(super) async fn adjustments(env: &TestEnv, sub_id: SubscriptionId) -> Vec<Invoice> {
    env.get_invoices(sub_id)
        .await
        .into_iter()
        .filter(|invoice| invoice.invoice_type == InvoiceType::Adjustment)
        .collect()
}

/// Starter's seats stored as the previous releases stored them: a fee of no price, its unit
/// alone (no quantity). It counts the quantity coded "Seats", the catalog's: 4 seats bought on
/// Jan 2 make 5, and Feb 1 bills 2900 + 5 × 1000 = 7900, as before.
/// Rules: R-137, I32
#[rstest]
#[tokio::test]
async fn a_seat_snapshot_without_its_quantity_counts_the_quantity_of_its_unit(
    #[future] test_env: TestEnv,
) {
    use diesel_async::RunQueryDsl;
    let env = test_env.await;
    let sub_id = on_starter(&env).await;
    diesel::sql_query(
        "UPDATE subscription_component SET price_id = NULL, legacy_fee = $1
         WHERE subscription_id = $2 AND price_component_id = $3",
    )
    .bind::<diesel::sql_types::Jsonb, _>(serde_json::json!({
        "Slot": {
            "unit": "Seats",
            "unit_rate": "10.00",
            "min_slots": 1,
            "max_slots": null,
            "initial_slots": 1
        }
    }))
    .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(COMP_STARTER_SEATS_ID.as_uuid())
    .execute(&mut env.conn().await)
    .await
    .expect("an old seat snapshot");

    change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, 4, day(1, 2)).await;
    assert_eq!(
        seats_of(&env, sub_id, QUANTITY_SEATS_ID, day(1, 3)).await,
        5
    );
    env.process_cycles().await;
    assert_eq!(renewal_on(&env, sub_id, day(2, 1)).await.total, 7_900);
}

/// A charge whose structure was backfilled from its product names the quantity coded as the
/// product's unit; one stored before (no `quantity_id` on the charge) bills the same: the
/// quantity is found by its code.
/// Rules: R-137, R-139
#[rstest]
#[tokio::test]
async fn a_seat_charge_naming_no_quantity_bills_as_one_naming_it(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;
    let env = test_env.await;
    diesel::sql_query(
        "UPDATE price_component SET fee_structure = fee_structure - 'quantity_id' WHERE id = $1",
    )
    .bind::<diesel::sql_types::Uuid, _>(COMP_STARTER_SEATS_ID.as_uuid())
    .execute(&mut env.conn().await)
    .await
    .expect("a charge stored as its product was");
    let sub_id = on_starter(&env).await;

    change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, 4, day(1, 2)).await;
    assert_eq!(
        seats_of(&env, sub_id, QUANTITY_SEATS_ID, day(1, 3)).await,
        5
    );
    env.process_cycles().await;
    assert_eq!(renewal_on(&env, sub_id, day(2, 1)).await.total, 7_900);
}

/// Two seat charges labelled "Seats" alike, one counting the catalog's seats at 10.00, the other
/// a quantity of editors at 5.00: each count changes alone. 4 seats bought on Jan 2 and 2 editors
/// on Jan 3, from 1 each: Feb 1 bills 2900 + 5 × 1000 + 3 × 500 = 9400. Before quantities, two
/// fees of one unit shared a count.
/// Rules: R-137
#[rstest]
#[tokio::test]
async fn seat_charges_of_one_label_count_their_own_quantities(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let editors = quantity(&env, "editors").await;
    let editor_product = product(&env, "Editor licences").await;
    let (version, seats, editor_seats) = (
        PlanVersionId::new(),
        PriceComponentId::new(),
        PriceComponentId::new(),
    );
    PlanSeed::new(PlanId::new(), "Two counts", version)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(2900, 2),
            ),
            SeedComp::slot(
                seats,
                "Seats",
                PRODUCT_SEATS_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(1000, 2),
            ),
            SeedComp::slot(
                editor_seats,
                "Editors",
                editor_product,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(500, 2),
            )
            .with_structure(seats_counting(editors)),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    let sub_id = subscription()
        .plan_version(version)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    change_seats(&env, sub_id, seats, 4, day(1, 2)).await;
    change_seats(&env, sub_id, editor_seats, 2, day(1, 3)).await;
    assert_eq!(
        seats_of(&env, sub_id, QUANTITY_SEATS_ID, day(1, 4)).await,
        5
    );
    assert_eq!(seats_of(&env, sub_id, editors, day(1, 4)).await, 3);

    env.process_cycles().await;
    assert_eq!(renewal_on(&env, sub_id, day(2, 1)).await.total, 9_400);
}

/// "Seat support", 2.00 a seat of the catalog's seats, an add-on charging per seat.
pub(super) async fn seat_support(env: &TestEnv, max_instances: Option<i32>) -> AddOnId {
    seat_add_on(
        env,
        "Seat support",
        BillingPeriodEnum::Monthly,
        dec!(2),
        (None, None),
        max_instances,
    )
    .await
}

/// An add-on charging `rate` a seat of the catalog's seats on `cadence`, within `bounds`.
async fn seat_add_on(
    env: &TestEnv,
    name: &str,
    cadence: BillingPeriodEnum,
    rate: Decimal,
    bounds: (Option<u32>, Option<u32>),
    max_instances: Option<i32>,
) -> AddOnId {
    env.store()
        .create_add_on_with_prices(
            name.to_string(),
            ProductRef::New {
                name: name.to_string(),
                fee_type: FeeTypeEnum::Slot,
                fee_structure: seats_counting(QUANTITY_SEATS_ID),
            },
            None,
            vec![PriceEntry::New(PriceInput {
                cadence,
                currency: "EUR".to_string(),
                pricing: Pricing::Slot {
                    model: RatingModel::PerUnit { rate },
                    min_slots: bounds.0,
                    max_slots: bounds.1,
                },
                credit_unit: None,
            })],
            None,
            true,
            max_instances,
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await
        .expect("a seat add-on")
        .id
}

/// A seat change that may be refused, its refusal as text.
pub(super) async fn try_change_seats(
    env: &TestEnv,
    sub_id: SubscriptionId,
    charge: impl Into<meteroid_store::domain::SeatCharge>,
    delta: i32,
    on: NaiveDate,
) -> Result<(), String> {
    env.services()
        .update_subscription_slots_for_test(
            TENANT_ID,
            sub_id,
            charge,
            delta,
            SlotUpgradeBillingMode::Optimistic,
            Some(on.and_time(NaiveTime::MIN)),
        )
        .await
        .map(|_| ())
        .map_err(|err| format!("{err:?}"))
}

/// The add-on added on `on`, at once.
pub(super) async fn add_on_at(
    env: &TestEnv,
    sub_id: SubscriptionId,
    add_on_id: AddOnId,
    on: NaiveDate,
) {
    env.services()
        .apply_amendment_immediate_at(Actor::System, sub_id, TENANT_ID, add(add_on_id, 1), on)
        .await
        .expect("add the seat add-on");
}

fn add(add_on_id: AddOnId, instances: i32) -> SubscriptionAmendment {
    SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges::default(),
        add_on_changes: AddOnChanges {
            added: vec![CreateSubscriptionAddOn {
                add_on_id,
                customization: SubscriptionAddOnCustomization::None,
                quantity: instances,
            }],
            edited: vec![],
            removed: vec![],
        },
    }
}

/// Starter with 5 seats (1, and 4 bought on Jan 2); "Seat support" at 2.00 a seat of the same
/// quantity added on Jan 16: it binds to the 5 seats, no count of its own, and bills them for
/// the rest of January, 5 × 200 × 16/31 = 516.13 → 516. Feb 1 bills 2900 + 5 × 1000 + 5 × 200 =
/// 8900. A seat bought through the add-on on Feb 10 is a seat of both charges, 6, priced on each
/// for the 20 days left of February's 29, Feb 10 included: 6000 × 20/29 = 4137.93 → 4138 less
/// 5000 × 20/29 = 3448.28 → 3448, 690, and 1200 × 20/29 = 827.59 → 828 less 1000 × 20/29 =
/// 689.66 → 690, 138; 828 in all. Mar 1 bills 2900 + 6 × 1000 + 6 × 200 = 10100.
/// Rules: R-137, R-138, R-079
#[rstest]
#[tokio::test]
async fn a_seat_add_on_shares_the_plans_count_and_bills_it(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let support = seat_support(&env, None).await;
    let sub_id = on_starter(&env).await;
    change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, 4, day(1, 2)).await;

    env.services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            add(support, 1),
            day(1, 16),
        )
        .await
        .expect("add the seat add-on");
    assert_eq!(
        seats_of(&env, sub_id, QUANTITY_SEATS_ID, day(1, 17)).await,
        5,
        "the add-on sets no count of its own"
    );
    let bought = adjustments(&env, sub_id).await;
    let add_on_stub = bought
        .iter()
        .find(|invoice| {
            invoice
                .line_items
                .iter()
                .any(|l| l.name.contains("Seat support"))
        })
        .expect("the add-on's adjustment");
    assert_eq!(add_on_stub.total, 516, "{:?}", add_on_stub.line_items);

    env.process_cycles().await;
    assert_eq!(renewal_on(&env, sub_id, day(2, 1)).await.total, 8_900);

    let before = adjustments(&env, sub_id).await.len();
    change_seats(&env, sub_id, support, 1, day(2, 10)).await;
    assert_eq!(
        seats_of(&env, sub_id, QUANTITY_SEATS_ID, day(2, 11)).await,
        6
    );
    let purchase = adjustments(&env, sub_id).await;
    assert_eq!(purchase.len(), before + 1, "one adjustment for the seat");
    let purchase = purchase
        .iter()
        .find(|invoice| {
            invoice
                .line_items
                .iter()
                .all(|l| l.start_date == day(2, 10))
        })
        .expect("the purchase");
    assert_eq!(purchase.line_items.len(), 2, "{:?}", purchase.line_items);
    assert_eq!(purchase.total, 828, "{:?}", purchase.line_items);

    env.process_cycles().await;
    assert_eq!(renewal_on(&env, sub_id, day(3, 1)).await.total, 10_100);
}

/// A seat add-on's rows count the seats it holds, not its one instance: "Seat support" granting 2
/// projects a seat, on Starter's 5 seats, adds 10 to the default's 1. Written without an effect,
/// its row adds.
/// Rules: R-138
#[rstest]
#[tokio::test]
async fn a_seat_add_on_grants_per_seat_held(#[future] test_env: TestEnv) {
    use common_domain::ids::{AliasOr, EntitlementEntityId};
    use meteroid_store::domain::entitlement_checks::CheckRequest;
    use meteroid_store::domain::entitlement_fold::{Effect, GrantValue, LimitValue};
    use meteroid_store::domain::entitlements::{
        EntitlementNew, EntitlementSpec, FeatureKind, FeatureNew, LimitCountedBy,
    };
    use meteroid_store::repositories::entitlements::EntitlementsInterface;

    let env = test_env.await;
    let support = seat_support(&env, None).await;
    let sub_id = on_starter(&env).await;
    change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, 4, day(1, 2)).await;
    add_on_at(&env, sub_id, support, day(1, 16)).await;
    let limit = |n: i64| GrantValue::Limit(LimitValue::Amount(Decimal::from(n)));
    let projects = env
        .store()
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: None,
                name: "Projects".to_string(),
                code: "projects".to_string(),
                description: None,
                kind: FeatureKind::Limit(LimitCountedBy::App),
                unit: None,
                level_type_id: None,
                policy: None,
                default: Some(limit(1)),
            },
        )
        .await
        .expect("a limit")
        .id;
    let row = env
        .store()
        .create_entitlement(
            &Actor::System,
            EntitlementNew {
                tenant_id: TENANT_ID,
                entity: EntitlementEntityId::AddOn(support),
                resource_id: None,
                spec: EntitlementSpec::new(projects, limit(2)),
            },
        )
        .await
        .expect("the add-on's row");
    assert_eq!(row.effect, Effect::Add);

    let checked = env
        .store()
        .check_entitlement(
            TENANT_ID,
            CUST_UBER_ID,
            AliasOr::Id(projects),
            CheckRequest {
                current_count: Some(Decimal::ZERO),
                ..CheckRequest::default()
            },
        )
        .await
        .expect("checked");
    assert_eq!(
        checked.value,
        Some(limit(11)),
        "1 + 2 for each of the 5 seats: {checked:?}"
    );
}

/// A seat add-on is one instance at most: created to allow more is refused, a subscription
/// asking two instances is refused.
/// Rules: R-138
#[rstest]
#[tokio::test]
async fn a_seat_add_on_has_one_instance(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let refused = env
        .store()
        .create_add_on_with_prices(
            "Seat support x10".to_string(),
            ProductRef::New {
                name: "Seat support x10".to_string(),
                fee_type: FeeTypeEnum::Slot,
                fee_structure: seats_counting(QUANTITY_SEATS_ID),
            },
            None,
            vec![PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: Pricing::Slot {
                    model: RatingModel::PerUnit { rate: dec!(2) },
                    min_slots: None,
                    max_slots: None,
                },
                credit_unit: None,
            })],
            None,
            true,
            Some(10),
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
        .await;
    assert!(
        refused.is_err_and(|err| format!("{err:?}").contains("one instance at most")),
        "ten instances of a seat add-on"
    );

    let support = seat_support(&env, Some(1)).await;
    let sub_id = on_starter(&env).await;
    let refused = env
        .services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            add(support, 2),
            day(1, 16),
        )
        .await;
    assert!(refused.is_err(), "two instances of a seat add-on");
}

/// One product charged two ways: "Platform" a flat 50.00 in Basic, 10.00 a seat in Business. A
/// change from Basic to Business does not continue the flat fee as seats: it removes it and adds
/// the seat charge, which starts at its minimum, 1 seat, its quantity not held: Feb 1 bills 1000.
/// Rules: R-139, R-108
#[rstest]
#[tokio::test]
async fn a_product_charged_flat_in_one_plan_and_per_seat_in_another(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let platform = product(&env, "Platform").await;
    let (basic, business) = (PlanVersionId::new(), PlanVersionId::new());
    PlanSeed::new(PlanId::new(), "Basic", basic)
        .components(vec![SeedComp::rate(
            PriceComponentId::new(),
            "Platform",
            platform,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            Decimal::new(5000, 2),
        )])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    PlanSeed::new(PlanId::new(), "Business", business)
        .components(vec![
            SeedComp::slot(
                PriceComponentId::new(),
                "Platform",
                platform,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(1000, 2),
            )
            .with_structure(seats_counting(QUANTITY_SEATS_ID)),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    let sub_id = subscription()
        .plan_version(basic)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;
    let january = env
        .get_invoices(sub_id)
        .await
        .into_iter()
        .find(|invoice| invoice.billing_period_start == day(1, 1))
        .expect("the opening invoice");
    assert_eq!(january.total, 5_000);

    let preview = env
        .services()
        .preview_plan_change(sub_id, TENANT_ID, business, vec![], None)
        .await
        .expect("a preview");
    assert!(
        preview.preview.matched.is_empty(),
        "a flat fee is not continued as seats"
    );
    assert_eq!(preview.preview.removed.len(), 1);
    assert_eq!(preview.preview.added.len(), 1);

    env.services()
        .schedule_plan_change(Actor::System, sub_id, TENANT_ID, business, vec![])
        .await
        .expect("schedule the change");
    env.process_cycles().await;
    assert_eq!(env.get_subscription(sub_id).await.plan_version_id, business);
    assert_eq!(
        seats_of(&env, sub_id, QUANTITY_SEATS_ID, day(2, 1)).await,
        1
    );
    assert_eq!(renewal_on(&env, sub_id, day(2, 1)).await.total, 1_000);
}

/// Starter's seats at 10.00 a month and "Yearly seat support" at 24.00 a seat a year counting the
/// same quantity, added on Jan 16, with 5 seats (1, and 4 bought on Jan 2). 2 seats removed on
/// Feb 10 for the period's end leave on Mar 1, the earliest end of the charges counting them (the
/// month's), whichever charge the removal is asked through: Mar 1 bills 2900 + 3 × 1000 = 5900,
/// nothing given back. The year keeps billing 5 seats to its first renewal on or after Mar 1, Jan
/// 1, 2025. Asked through the add-on, the removal was dated on the year's end and the month billed
/// the 2 seats until then, never given back: 2900 + 5 × 1000 = 7900 on Mar 1.
/// Rules: R-108, R-137
#[rstest]
#[case::through_the_add_on(true)]
#[case::through_the_plan_charge(false)]
#[tokio::test]
async fn a_removal_of_a_shared_count_leaves_at_the_earliest_period_end(
    #[future] test_env: TestEnv,
    #[case] through_the_add_on: bool,
) {
    let env = test_env.await;
    let yearly = seat_add_on(
        &env,
        "Yearly seat support",
        BillingPeriodEnum::Annual,
        dec!(24),
        (None, None),
        None,
    )
    .await;
    let sub_id = on_starter(&env).await;
    change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, 4, day(1, 2)).await;
    add_on_at(&env, sub_id, yearly, day(1, 16)).await;
    env.process_cycles().await;
    assert_eq!(renewal_on(&env, sub_id, day(2, 1)).await.total, 7_900);

    if through_the_add_on {
        change_seats(&env, sub_id, yearly, -2, day(2, 10)).await;
    } else {
        change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, -2, day(2, 10)).await;
    }
    assert_eq!(
        seats_of(&env, sub_id, QUANTITY_SEATS_ID, day(2, 29)).await,
        5,
        "billed until the month's end"
    );
    assert_eq!(
        seats_of(&env, sub_id, QUANTITY_SEATS_ID, day(3, 1)).await,
        3,
        "gone at the month's end, not the year's"
    );

    env.process_cycles().await;
    assert_eq!(renewal_on(&env, sub_id, day(3, 1)).await.total, 5_900);
}

/// Starter's seats (1 at least) and "Seat support 2-6" (2.00 a seat, 2 to 6 seats) count one
/// quantity: the strictest bounds hold, now and at the renewal. With 5 seats (1, and 4 bought on
/// Jan 2) and the add-on from Jan 16, 2 more through the plan charge would be 7, above the
/// add-on's 6: refused; 4 removed through it would leave 1 on Feb 1, below the add-on's 2:
/// refused. 3 removed on Jan 20 leave 2 on Feb 1; 1 more removed on Jan 21 through the add-on
/// would leave 1 there: refused, though 4 are held until then. 1 bought on Jan 22 makes 6 now and
/// 3 on Feb 1: Feb 1 bills 2900 + 3 × 1000 + 3 × 200 = 6500.
/// Rules: R-108, R-137
#[rstest]
#[tokio::test]
async fn the_strictest_bounds_of_the_charges_counting_a_quantity_hold(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let support = seat_add_on(
        &env,
        "Seat support 2-6",
        BillingPeriodEnum::Monthly,
        dec!(2),
        (Some(2), Some(6)),
        None,
    )
    .await;
    let sub_id = on_starter(&env).await;
    change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, 4, day(1, 2)).await;
    add_on_at(&env, sub_id, support, day(1, 16)).await;

    let above = try_change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, 2, day(1, 20)).await;
    assert!(
        above.as_ref().is_err_and(|err| err.contains("maximum")),
        "7 seats above the add-on's 6: {above:?}"
    );
    let below = try_change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, -4, day(1, 20)).await;
    assert!(
        below.as_ref().is_err_and(|err| err.contains("minimum")),
        "1 seat below the add-on's 2: {below:?}"
    );

    change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, -3, day(1, 20)).await;
    let at_renewal = try_change_seats(&env, sub_id, support, -1, day(1, 21)).await;
    assert!(
        at_renewal
            .as_ref()
            .is_err_and(|err| err.contains("minimum")),
        "1 seat at the renewal: {at_renewal:?}"
    );

    change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, 1, day(1, 22)).await;
    assert_eq!(
        seats_of(&env, sub_id, QUANTITY_SEATS_ID, day(1, 23)).await,
        6
    );
    assert_eq!(
        seats_of(&env, sub_id, QUANTITY_SEATS_ID, day(2, 1)).await,
        3
    );
    env.process_cycles().await;
    assert_eq!(renewal_on(&env, sub_id, day(2, 1)).await.total, 6_500);
}

/// A seat add-on is held once by a subscription: added again while held, added twice in one
/// amendment, or twice at creation, it is refused, as a second one would bill every seat again.
/// Removed and added back in one amendment, it is held once.
/// Rules: R-138
#[rstest]
#[tokio::test]
async fn a_seat_add_on_is_held_once(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let support = seat_support(&env, None).await;
    let held_once =
        |result: Result<(), String>| result.is_err_and(|err| err.contains("holds it once"));

    let sub_id = on_starter(&env).await;
    add_on_at(&env, sub_id, support, day(1, 16)).await;
    let again = env
        .services()
        .apply_amendment_immediate_at(
            Actor::System,
            sub_id,
            TENANT_ID,
            add(support, 1),
            day(1, 20),
        )
        .await
        .map(|_| ())
        .map_err(|err| format!("{err:?}"));
    assert!(held_once(again), "added again while held");

    let twice = |add_on_id| SubscriptionAmendment {
        apply_mode: PlanChangeMode::Immediate,
        component_changes: ComponentChanges::default(),
        add_on_changes: AddOnChanges {
            added: vec![
                CreateSubscriptionAddOn {
                    add_on_id,
                    customization: SubscriptionAddOnCustomization::None,
                    quantity: 1,
                },
                CreateSubscriptionAddOn {
                    add_on_id,
                    customization: SubscriptionAddOnCustomization::None,
                    quantity: 1,
                },
            ],
            edited: vec![],
            removed: vec![],
        },
    };
    let other = on_starter(&env).await;
    let in_one_amendment = env
        .services()
        .apply_amendment_immediate_at(Actor::System, other, TENANT_ID, twice(support), day(1, 16))
        .await
        .map(|_| ())
        .map_err(|err| format!("{err:?}"));
    assert!(held_once(in_one_amendment), "added twice in one amendment");

    let mut request = subscription()
        .plan_version(PLAN_VERSION_STARTER_ID)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .request();
    request.add_ons = Some(CreateSubscriptionAddOns {
        add_ons: twice(support).add_on_changes.added,
    });
    let at_creation = env
        .services()
        .insert_subscription(Actor::System, request, TENANT_ID)
        .await
        .map(|_| ())
        .map_err(|err| format!("{err:?}"));
    assert!(held_once(at_creation), "twice at creation");

    // Removed and added back: held once.
    let held = env
        .store()
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("the subscription")
        .add_ons
        .into_iter()
        .find(|a| a.add_on_id == support)
        .expect("the add-on held")
        .id;
    let mut swap = add(support, 1);
    swap.add_on_changes.removed = vec![held];
    env.services()
        .apply_amendment_immediate_at(Actor::System, sub_id, TENANT_ID, swap, day(1, 25))
        .await
        .expect("removed and added back");
}

/// Starter with 5 seats (1, and 4 bought on Jan 2) and "Seat support", 2.00 a seat of the same
/// quantity, from Jan 16. A seat bought through the plan charge on Feb 10 is a seat of both: the
/// MRR reads 6 on each, 2900 + 6 × 1000 + 6 × 200 = 10100, where the add-on's line read the seats
/// it was added with.
/// Rules: R-137, R-138
#[rstest]
#[tokio::test]
async fn a_seat_add_on_s_mrr_reads_the_live_count(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let support = seat_support(&env, None).await;
    let sub_id = on_starter(&env).await;
    change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, 4, day(1, 2)).await;
    add_on_at(&env, sub_id, support, day(1, 16)).await;
    env.process_cycles().await;
    assert_eq!(renewal_on(&env, sub_id, day(2, 1)).await.total, 8_900);

    change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, 1, day(2, 10)).await;
    assert_eq!(
        env.get_subscription(sub_id).await.mrr_cents,
        10_100,
        "6 seats on the plan charge and on the add-on"
    );
}

/// Starter with 5 seats (1, and 4 bought on Jan 2), a change scheduled for Feb 1 to "Editors"
/// (29.00 and 25.00 an editor, a quantity Starter does not count) setting 3 editors: the editors
/// open at 3 on Feb 1, the catalog's seats are no count of the target. Feb 1 bills 2900 + 3 ×
/// 2500 = 10400.
/// Rules: R-108, R-137
#[rstest]
#[tokio::test]
async fn a_scheduled_change_sets_the_count_of_a_quantity_not_held(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let editors = quantity(&env, "editors").await;
    let editor_product = product(&env, "Editor licences").await;
    let (version, editor_seats) = (PlanVersionId::new(), PriceComponentId::new());
    PlanSeed::new(PlanId::new(), "Editors", version)
        .components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(2900, 2),
            ),
            SeedComp::slot(
                editor_seats,
                "Editors",
                editor_product,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(2500, 2),
            )
            .with_structure(seats_counting(editors)),
        ])
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    let sub_id = on_starter(&env).await;
    change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, 4, day(1, 2)).await;

    env.services()
        .schedule_plan_change(
            Actor::System,
            sub_id,
            TENANT_ID,
            version,
            vec![ComponentParameterization {
                component_id: editor_seats,
                parameters: ComponentParameters {
                    initial_slot_count: Some(3),
                    billing_period: None,
                    committed_capacity: None,
                },
            }],
        )
        .await
        .expect("schedule the change");
    env.process_cycles().await;

    assert_eq!(env.get_subscription(sub_id).await.plan_version_id, version);
    assert_eq!(seats_of(&env, sub_id, editors, day(2, 1)).await, 3);
    assert_eq!(renewal_on(&env, sub_id, day(2, 1)).await.total, 10_400);
}

/// A subscription of a v1 plan after the migration: its plan charge has no structure, and its
/// seat snapshot names no quantity, its unit spelt "seats" where the catalog's quantity is coded
/// "Seats". It counts that quantity, its unit read whatever its case: 4 seats bought on Jan 2
/// make 5, Feb 1 bills 2900 + 5 × 1000 = 7900. Read by its exact code, the unit named no quantity
/// and the change was refused.
/// Rules: R-137, I32
#[rstest]
#[tokio::test]
async fn a_v1_seat_subscription_changes_its_seats_after_the_migration(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;
    let env = test_env.await;
    let sub_id = on_starter(&env).await;
    diesel::sql_query("UPDATE price_component SET fee_structure = NULL WHERE id = $1")
        .bind::<diesel::sql_types::Uuid, _>(COMP_STARTER_SEATS_ID.as_uuid())
        .execute(&mut env.conn().await)
        .await
        .expect("a v1 charge");
    diesel::sql_query(
        "UPDATE subscription_component SET price_id = NULL, legacy_fee = $1
         WHERE subscription_id = $2 AND price_component_id = $3",
    )
    .bind::<diesel::sql_types::Jsonb, _>(serde_json::json!({
        "Slot": {
            "unit": "seats",
            "unit_rate": "10.00",
            "min_slots": 1,
            "max_slots": null,
            "initial_slots": 1
        }
    }))
    .bind::<diesel::sql_types::Uuid, _>(sub_id.as_uuid())
    .bind::<diesel::sql_types::Uuid, _>(COMP_STARTER_SEATS_ID.as_uuid())
    .execute(&mut env.conn().await)
    .await
    .expect("a v1 seat snapshot");

    change_seats(&env, sub_id, COMP_STARTER_SEATS_ID, 4, day(1, 2)).await;
    assert_eq!(
        seats_of(&env, sub_id, QUANTITY_SEATS_ID, day(1, 3)).await,
        5
    );
    env.process_cycles().await;
    assert_eq!(renewal_on(&env, sub_id, day(2, 1)).await.total, 7_900);
}

/// A charge naming its seats by unit alone (REST v1, a client omitting the quantity) counts the
/// quantity that unit names whatever its case and spacing: " seats " is the catalog's "Seats",
/// never a second count. A new unit makes the code "New quantity…" would: "Editor Seats" is
/// `editor-seats`.
/// Rules: R-137, I32
#[rstest]
#[tokio::test]
async fn a_unit_names_one_quantity_whatever_its_spelling(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let by_unit = |unit: &str| FeeStructure::Slot {
        unit_name: unit.to_string(),
        quantity_id: None,
        upgrade_policy: UpgradePolicy::Prorated,
        downgrade_policy: DowngradePolicy::RemoveAtEndOfPeriod,
    };
    let quantity_of = |add_on: meteroid_store::domain::add_ons::AddOn| match add_on.fee_structure {
        Some(FeeStructure::Slot { quantity_id, .. }) => quantity_id,
        other => panic!("a seat structure, not {other:?}"),
    };
    let store = env.store();
    let create = move |name: &str, unit: &str| {
        store.create_add_on_with_prices(
            name.to_string(),
            ProductRef::New {
                name: name.to_string(),
                fee_type: FeeTypeEnum::Slot,
                fee_structure: by_unit(unit),
            },
            Some(by_unit(unit)),
            vec![PriceEntry::New(PriceInput {
                cadence: BillingPeriodEnum::Monthly,
                currency: "EUR".to_string(),
                pricing: Pricing::Slot {
                    model: RatingModel::PerUnit { rate: dec!(2) },
                    min_slots: None,
                    max_slots: None,
                },
                credit_unit: None,
            })],
            None,
            true,
            None,
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            vec![],
        )
    };

    let lowercase = create("Seat support", " seats ").await.expect("an add-on");
    assert_eq!(quantity_of(lowercase), Some(QUANTITY_SEATS_ID));

    let new = create("Editor support", "Editor Seats")
        .await
        .expect("an add-on");
    let new = quantity_of(new).expect("a quantity");
    let row = ManagedQuantityRow::find(&mut env.conn().await, TENANT_ID, new)
        .await
        .expect("a read")
        .expect("the new quantity");
    assert_eq!(row.code, "editor-seats");
    assert_eq!(row.name, "Editor Seats");
    let again = create("Editor support 2", "editor seats")
        .await
        .expect("an add-on");
    assert_eq!(quantity_of(again), Some(new), "one count for one unit");
}

/// REST v1 creates an add-on from a product and a price alone: its structure is the price's
/// charges' (the Starter seat price: seats of the catalog's quantity), else the product's stored
/// one when it fits the price (a seat product of unit "seats", normalised to the catalog's
/// "Seats"), else the pricing's alone.
/// Rules: R-139, R-137
#[rstest]
#[tokio::test]
async fn a_v1_add_on_takes_the_structure_its_price_tells(#[future] test_env: TestEnv) {
    use diesel_async::RunQueryDsl;
    let env = test_env.await;
    let of_starter = env
        .store()
        .add_on_structure_of_price(TENANT_ID, PRODUCT_SEATS_ID, PRICE_STARTER_SEATS_ID)
        .await
        .expect("a structure");
    assert!(
        matches!(
            of_starter,
            Some(FeeStructure::Slot {
                quantity_id: Some(q),
                ..
            }) if q == QUANTITY_SEATS_ID
        ),
        "{of_starter:?}"
    );

    // A product of the previous release, its structure stored on it, and a seat price no charge
    // uses yet.
    let licences = product(&env, "Licences").await;
    diesel::sql_query("UPDATE product SET fee_structure = $1 WHERE id = $2")
        .bind::<diesel::sql_types::Jsonb, _>(serde_json::json!({
            "type": "Slot",
            "unit_name": "seats",
            "upgrade_policy": "Prorated",
            "downgrade_policy": "RemoveAtEndOfPeriod"
        }))
        .bind::<diesel::sql_types::Uuid, _>(licences.as_uuid())
        .execute(&mut env.conn().await)
        .await
        .expect("a stored structure");
    let price = PriceId::new();
    diesel_models::prices::PriceRowNew {
        id: price,
        product_id: licences,
        cadence: DieselBillingPeriodEnum::Monthly,
        currency: "EUR".to_string(),
        pricing: serde_json::to_value(Pricing::Slot {
            model: RatingModel::PerUnit { rate: dec!(3) },
            min_slots: None,
            max_slots: None,
        })
        .unwrap(),
        tenant_id: TENANT_ID,
        catalog: true,
        credit_unit_id: None,
        credit_unit_rate: None,
    }
    .insert(&mut env.conn().await)
    .await
    .expect("a seat price");

    let structure = env
        .store()
        .add_on_structure_of_price(TENANT_ID, licences, price)
        .await
        .expect("a structure")
        .expect("told by the product");
    let add_on = env
        .store()
        .create_add_on(
            Actor::System,
            AddOnNew {
                name: "Licences".to_string(),
                tenant_id: TENANT_ID,
                product_id: licences,
                price_id: price,
                fee_structure: structure,
                description: None,
                self_serviceable: true,
                max_instances_per_subscription: None,
                entitlements: vec![],
            },
        )
        .await
        .expect("a v1 add-on");
    assert!(
        matches!(
            add_on.fee_structure,
            Some(FeeStructure::Slot {
                quantity_id: Some(q),
                ..
            }) if q == QUANTITY_SEATS_ID
        ),
        "{:?}",
        add_on.fee_structure
    );
}

/// One product metered two ways: "Compute" billed on bandwidth in one plan and on database size
/// in the other, each plan with the same 10.00 platform fee. A change between them continues the
/// platform fee only: the compute charge is removed and added, its usage read on its own metric.
/// Before, charges of one kind matched whatever their metric.
/// Rules: R-139
#[rstest]
#[tokio::test]
async fn usage_of_another_metric_is_another_charge(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let compute = product(&env, "Compute").await;
    let plan = |name: &'static str, version: PlanVersionId, metric| {
        PlanSeed::new(PlanId::new(), name, version).components(vec![
            SeedComp::rate(
                PriceComponentId::new(),
                "Platform Fee",
                PRODUCT_PLATFORM_FEE_ID,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                Decimal::new(1000, 2),
            ),
            SeedComp::usage(
                PriceComponentId::new(),
                "Compute",
                compute,
                metric,
                PriceId::new(),
                DieselBillingPeriodEnum::Monthly,
                UsagePricingModel::PerUnit {
                    rate: Decimal::new(10, 2),
                },
            ),
        ])
    };
    let (on_bandwidth, on_storage) = (PlanVersionId::new(), PlanVersionId::new());
    plan("On bandwidth", on_bandwidth, METRIC_BANDWIDTH)
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    plan("On storage", on_storage, METRIC_DATABASE_SIZE)
        .seed(&mut env.conn().await)
        .await
        .unwrap();
    let sub_id = subscription()
        .plan_version(on_bandwidth)
        .start_date(day(1, 1))
        .on_start()
        .no_trial()
        .create(env.services())
        .await;

    let preview = env
        .services()
        .preview_plan_change(sub_id, TENANT_ID, on_storage, vec![], None)
        .await
        .expect("a preview");
    assert_eq!(preview.preview.matched.len(), 1, "the platform fee alone");
    assert_eq!(preview.preview.removed.len(), 1, "compute on bandwidth");
    assert_eq!(preview.preview.added.len(), 1, "compute on database size");
}
