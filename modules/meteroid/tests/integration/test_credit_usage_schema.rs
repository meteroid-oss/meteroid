//! Storage behind usage settlement and credit draws: what the schema itself guarantees.
use crate::data::ids::{CUST_UBER_ID, PLAN_VERSION_1_LEETCODE_ID, TENANT_ID};
use crate::helpers::credit::{fund, wallets};
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use crate::test_customer_credit::{draft, finalize, line};
use chrono::{DateTime, Duration, NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{BaseId, SubscriptionId, UsageSettlementId, WalletId};
use diesel::sql_types::{Text, Uuid as SqlUuid};
use diesel_async::RunQueryDsl;
use diesel_models::customer_credit::{CreditUsageSettlementRow, CreditUsageSettlementRowNew};
use meteroid_store::domain::enums::InvoiceStatusEnum;
use meteroid_store::domain::{
    CreateSubscription, SubscriptionActivationCondition, SubscriptionNew,
};
use meteroid_store::repositories::InvoiceInterface;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn test_error(error: impl std::fmt::Debug) -> Box<dyn std::error::Error> {
    format!("{error:?}").into()
}

async fn subscription(setup: &MeteroidSetup) -> SubscriptionId {
    let today = Utc::now().date_naive();
    setup
        .services
        .insert_subscription(
            Actor::System,
            CreateSubscription {
                subscription: SubscriptionNew {
                    customer_id: CUST_UBER_ID,
                    plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
                    net_terms: None,
                    invoice_memo: None,
                    invoice_threshold: None,
                    start_date: today,
                    end_date: None,
                    billing_start_date: Some(today),
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
        .expect("a subscription")
        .id
}

async fn setup() -> MeteroidSetup {
    crate::helpers::init::logging();
    let database =
        crate::meteroid_it::container::create_seeded_test_database(SeedLevel::PLANS).await;
    crate::meteroid_it::container::start_meteroid(database, SeedLevel::PLANS).await
}

async fn wallet(setup: &MeteroidSetup) -> WalletId {
    fund(&setup.store, TENANT_ID, CUST_UBER_ID, None, 10_000).await;
    wallets(&setup.store, TENANT_ID, CUST_UBER_ID)
        .await
        .into_iter()
        .next()
        .expect("a wallet")
        .id
}

#[derive(diesel::QueryableByName)]
struct Text1 {
    #[diesel(sql_type = Text)]
    value: String,
}

async fn text(setup: &MeteroidSetup, sql: &str) -> TestResult<Vec<String>> {
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let rows: Vec<Text1> = diesel::sql_query(sql)
        .load(&mut conn)
        .await
        .map_err(test_error)?;
    Ok(rows.into_iter().map(|r| r.value).collect())
}

async fn rejected(setup: &MeteroidSetup, sql: &str, account_id: WalletId) -> TestResult<String> {
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let error = diesel::sql_query(sql)
        .bind::<SqlUuid, _>(account_id.as_uuid())
        .execute(&mut conn)
        .await
        .expect_err("the statement must be rejected");
    Ok(error.to_string())
}

#[tokio::test]
async fn usage_settled_at_a_bound_again_is_recorded_after_it_and_handed_to_its_invoice_once()
-> TestResult {
    let setup = setup().await;
    setup.skip_credit_invariants(
        "writes ledger, settlement or invoice rows by raw SQL for the revenue projector or the schema",
    );
    let account_id = wallet(&setup).await;
    let subscription_id = subscription(&setup).await;
    let period = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
    let bound = Utc::now();
    let settlement = |bound: DateTime<Utc>, amount: i64| CreditUsageSettlementRowNew {
        id: UsageSettlementId::new(),
        tenant_id: TENANT_ID,
        customer_id: CUST_UBER_ID,
        account_id,
        subscription_id,
        period_start: period,
        event_time_bound: bound,
        amount,
        tax_inclusive_rate: None,
        tax_minor: 0,
    };

    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    CreditUsageSettlementRow::insert_new(&mut conn, &settlement(bound, 300))
        .await
        .map_err(test_error)?
        .expect("recorded");
    // Late usage settled at the same bound: what the earlier one paid is netted by its caller.
    CreditUsageSettlementRow::insert_new(&mut conn, &settlement(bound, 20))
        .await
        .map_err(test_error)?
        .expect("recorded after the first");
    CreditUsageSettlementRow::insert_new(&mut conn, &settlement(bound + Duration::hours(1), 50))
        .await
        .map_err(test_error)?
        .expect("recorded");
    assert_eq!(
        CreditUsageSettlementRow::list_for_period(
            &mut conn,
            TENANT_ID,
            subscription_id,
            period,
            "EUR",
        )
        .await
        .map_err(test_error)?
        .len(),
        3
    );

    let invoice = draft(&setup, vec![line("Usage", 1_000, true)]).await?;
    let attached = CreditUsageSettlementRow::attach_open_to_invoice(
        &mut conn,
        TENANT_ID,
        subscription_id,
        period,
        &[account_id],
        invoice.id,
    )
    .await
    .map_err(test_error)?;
    assert_eq!(attached.len(), 3);
    assert!(
        CreditUsageSettlementRow::attach_open_to_invoice(
            &mut conn,
            TENANT_ID,
            subscription_id,
            period,
            &[account_id],
            invoice.id,
        )
        .await
        .map_err(test_error)?
        .is_empty(),
        "settlements are handed over once"
    );
    let by_invoice = CreditUsageSettlementRow::list_by_invoice(&mut conn, TENANT_ID, invoice.id)
        .await
        .map_err(test_error)?;
    assert_eq!(by_invoice.iter().map(|s| s.amount).sum::<i64>(), 370);
    drop(conn);

    let rejected_zero = rejected(
        &setup,
        "INSERT INTO credit_usage_settlement (id, tenant_id, customer_id, account_id, subscription_id,
            period_start, event_time_bound, amount, tax_minor, give_back_source_id)
         SELECT gen_random_uuid(), tenant_id, customer_id, id, (SELECT id FROM subscription LIMIT 1),
            '2026-08-01', now(), 0, 0, gen_random_uuid() FROM customer_credit_account WHERE id = $1",
        account_id,
    )
    .await?;
    assert!(rejected_zero.contains("check"), "{rejected_zero}");
    Ok(())
}

#[tokio::test]
async fn draws_record_what_paid_them() -> TestResult {
    let setup = setup().await;
    wallet(&setup).await;

    let invoice = draft(&setup, vec![line("Seats", 4_000, false)]).await?;
    finalize(&setup, invoice.id).await?;
    let drawn = text(
        &setup,
        "SELECT kind || ':' || source_kind AS value FROM customer_credit_entry
         WHERE kind IN ('DRAW', 'DRAW_REVERSAL') ORDER BY created_at, id",
    )
    .await?;
    assert_eq!(drawn, vec!["DRAW:INVOICE"]);

    let voided = setup
        .store
        .void_invoice(Actor::System, invoice.id, TENANT_ID)
        .await
        .map_err(test_error)?
        .invoice;
    assert_eq!(voided.status, InvoiceStatusEnum::Void);
    let after_void = text(
        &setup,
        "SELECT kind || ':' || source_kind AS value FROM customer_credit_entry
         WHERE kind IN ('DRAW', 'DRAW_REVERSAL') ORDER BY kind",
    )
    .await?;
    assert_eq!(after_void, vec!["DRAW:INVOICE", "DRAW_REVERSAL:INVOICE"]);
    let funded = text(
        &setup,
        "SELECT coalesce(source_kind, 'none') AS value FROM customer_credit_entry WHERE kind = 'FUND'",
    )
    .await?;
    assert_eq!(funded, vec!["none"], "only draws carry a source");
    Ok(())
}
