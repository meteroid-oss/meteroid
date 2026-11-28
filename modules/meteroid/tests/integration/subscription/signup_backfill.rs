//! Customers with no plan in a product line, started on its free sign-up plan in bulk.
use chrono::Utc;
use common_domain::actor::Actor;
use common_domain::ids::{
    BaseId, BatchJobChunkId, CustomerId, PlanId, PlanVersionId, PriceComponentId, PriceId,
    SubscriptionId,
};
use common_domain::pgmq::{MessageReadQty, MessageReadVtSec};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use diesel_models::enums::{BillingPeriodEnum as DieselBillingPeriodEnum, SubscriptionStatusEnum};
use meteroid::workers::batch_jobs::engine::BatchJobProcessor;
use meteroid::workers::batch_jobs::processors::SignupPlanBackfillProcessor;
use meteroid_store::domain::batch_jobs::{BatchJob, BatchJobChunk, BatchJobNew};
use meteroid_store::domain::eligibility::Eligibility;
use meteroid_store::domain::enums::{BatchJobChunkStatusEnum, BatchJobTypeEnum, CustomerType};
use meteroid_store::domain::outbox_event::{OutboxEvent, SubscriptionOrigin};
use meteroid_store::domain::pgmq::PgmqQueue;
use meteroid_store::domain::prices::{FeeStructure, Pricing};
use meteroid_store::domain::product_family_journeys::ProductFamilyJourney;
use meteroid_store::domain::{CustomerNew, CustomerTaxStatus};
use meteroid_store::repositories::batch_jobs::BatchJobsInterface;
use meteroid_store::repositories::pgmq::PgmqInterface;
use meteroid_store::repositories::{
    CustomersInterface, EligibilityInterface, ProductFamilyJourneysInterface,
};
use meteroid_store::services::signup_backfill::BackfillOutcome;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde_json::json;

use crate::data::ids::*;
use crate::data::plans::{PlanSeed, SeedComp};
use crate::harness::{SubscriptionAssertExt, TestEnv, test_env};

/// A standard plan of a platform fee at `fee` a month, `trial` days free, the sign-up plan.
async fn signup_plan(env: &TestEnv, fee: Decimal, trial: Option<i32>) -> PlanId {
    let (plan, version) = (PlanId::new(), PlanVersionId::new());
    let mut seed = PlanSeed::new(plan, "Hobby", version).components(vec![SeedComp::rate(
        PriceComponentId::new(),
        "Platform",
        PRODUCT_PLATFORM_FEE_ID,
        PriceId::new(),
        DieselBillingPeriodEnum::Monthly,
        fee,
    )]);
    if let Some(days) = trial {
        seed = seed.trial(days, plan, true);
    }
    seed.seed(&mut env.conn().await).await.expect("a plan");
    env.store()
        .save_product_family_journey(
            TENANT_ID,
            ProductFamilyJourney {
                product_family_id: PRODUCT_FAMILY_ID,
                signup_plan_id: Some(plan),
                signup_option: None,
                period_switch: true,
                self_serve_downgrade: false,
            },
        )
        .await
        .expect("the sign-up plan");
    plan
}

async fn customer_in(env: &TestEnv, alias: &str, currency: &str) -> CustomerId {
    env.store()
        .insert_customer(
            Actor::System,
            CustomerNew {
                name: alias.to_string(),
                alias: Some(alias.to_string()),
                billing_email: Some(format!("{alias}@example.com")),
                invoicing_emails: vec![],
                phone: None,
                currency: currency.to_string(),
                billing_address: None,
                shipping_address: None,
                invoicing_entity_id: None,
                force_created_date: None,
                vat_number: None,
                legal_number: None,
                buyer_reference: None,
                custom_taxes: vec![],
                tax_status: CustomerTaxStatus::Taxable,
                exemption_reason: None,
                connected_account_id: None,
                custom_properties: json!({}),
                preferred_locales: vec![],
                customer_type: CustomerType::Company,
                first_name: None,
                last_name: None,
            },
            TENANT_ID,
        )
        .await
        .expect("a customer")
        .id
}

/// A job of the backfill, stored: the subscriptions it starts are recorded as its own.
async fn job(env: &TestEnv, plan_version_id: PlanVersionId) -> BatchJob {
    env.store()
        .create_batch_job(BatchJobNew {
            tenant_id: TENANT_ID,
            job_type: BatchJobTypeEnum::SignupPlanBackfill,
            input_source_key: None,
            input_params: Some(json!({
                "product_family_id": PRODUCT_FAMILY_ID,
                "plan_version_id": plan_version_id,
            })),
            file_hash: None,
            created_by: USER_ID.as_uuid(),
            input_file_name: None,
        })
        .await
        .expect("a backfill job")
}

/// The subscriptions the job recorded as its own.
async fn recorded(env: &TestEnv, job: &BatchJob) -> Vec<SubscriptionId> {
    use diesel_models::schema::batch_job_entity::dsl as e;
    e::batch_job_entity
        .filter(e::batch_job_id.eq(job.id))
        .filter(e::entity_type.eq("subscription"))
        .select(e::entity_id)
        .load::<uuid::Uuid>(&mut env.conn().await)
        .await
        .expect("the job's entities")
        .into_iter()
        .map(SubscriptionId::from)
        .collect()
}

fn chunk(job: &BatchJob, item_offset: i32, item_count: i32) -> BatchJobChunk {
    let now = Utc::now().naive_utc();
    BatchJobChunk {
        id: BatchJobChunkId::new(),
        job_id: job.id,
        tenant_id: TENANT_ID,
        chunk_index: 0,
        status: BatchJobChunkStatusEnum::Pending,
        item_offset,
        item_count,
        processed_count: 0,
        failed_count: 0,
        retry_count: 0,
        max_retries: 3,
        locked_at: None,
        retry_after: None,
        events: vec![],
        created_at: now,
        updated_at: now,
    }
}

/// The customers' subscriptions, by customer.
async fn subscribed(env: &TestEnv) -> Vec<CustomerId> {
    use diesel_models::schema::subscription::dsl as s;
    let mut ids: Vec<CustomerId> = s::subscription
        .filter(s::tenant_id.eq(TENANT_ID))
        .select(s::customer_id)
        .load(&mut env.conn().await)
        .await
        .expect("subscriptions");
    ids.sort();
    ids
}

/// The job's input file: the customer ids, one a line, under their header.
fn input_of(customers: &[CustomerId]) -> bytes::Bytes {
    std::iter::once("customer_id".to_owned())
        .chain(customers.iter().map(ToString::to_string))
        .map(|line| line + "\n")
        .collect::<String>()
        .into_bytes()
        .into()
}

/// The version of the product family's sign-up plan.
async fn signup_version(env: &TestEnv) -> PlanVersionId {
    env.services()
        .signup_backfill_customers(TENANT_ID, PRODUCT_FAMILY_ID, true)
        .await
        .expect("the sign-up plan")
        .0
}

async fn backfill(
    env: &TestEnv,
    version: PlanVersionId,
    customer: CustomerId,
) -> meteroid_store::StoreResult<BackfillOutcome> {
    env.services()
        .backfill_signup_plan(
            Actor::System,
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            version,
            None,
            customer,
        )
        .await
}

async fn created_origins(env: &TestEnv) -> Vec<Option<SubscriptionOrigin>> {
    env.store()
        .pgmq_read(
            PgmqQueue::WebhookOut,
            MessageReadQty(200),
            MessageReadVtSec(0),
        )
        .await
        .expect("read webhook_out queue")
        .into_iter()
        .filter_map(|m| m.message)
        .filter_map(|m| serde_json::from_value::<OutboxEvent>(m.0).ok())
        .filter_map(|event| match event {
            OutboxEvent::SubscriptionCreated(created) => Some(created.origin),
            _ => None,
        })
        .collect()
}

/// The previewed customers start on the free plan; one already on it, and one asking a
/// currency the plan is not sold in, are left off. Nothing reaches the customers' inboxes,
/// and running the job again starts no one twice.
#[tokio::test]
async fn a_backfill_starts_customers_with_no_plan_and_emails_no_one() {
    let env = test_env().await;
    signup_plan(&env, Decimal::ZERO, None).await;
    let offer = env
        .services()
        .signup_offer(TENANT_ID, Some(PRODUCT_FAMILY_ID), "EUR")
        .await
        .expect("the sign-up offer");
    env.services()
        .subscribe_to_signup_plan(Actor::System, TENANT_ID, CUST_UBER_ID, offer)
        .await
        .expect("Uber on the plan already");
    customer_in(&env, "dollars", "USD").await;

    let preview = env
        .services()
        .signup_backfill_preview(TENANT_ID, PRODUCT_FAMILY_ID, false)
        .await
        .expect("a preview");
    assert_eq!(
        (preview.customers, preview.not_sold, preview.churned),
        (2, 1, 0)
    );
    assert!(!preview.eligibility_rule);
    let (version, customers) = env
        .services()
        .signup_backfill_customers(TENANT_ID, PRODUCT_FAMILY_ID, false)
        .await
        .expect("the customers");
    let mut expected = vec![CUST_SPOTIFY_ID, CUST_COMODO_ID];
    expected.sort();
    assert_eq!(customers, expected);

    let processor = SignupPlanBackfillProcessor::new(env.services().clone());
    let job = job(&env, version).await;
    let input = input_of(&customers);
    let chunks = processor
        .prepare_chunks(&job, Some(input.clone()))
        .await
        .expect("chunks");
    assert_eq!(chunks.len(), 1);
    let result = processor
        .process_chunk(&job, &chunk(&job, 0, 2), Some(input.clone()))
        .await
        .expect("the chunk");
    assert!(result.failures.is_empty(), "{:?}", result.failures);
    let started = recorded(&env, &job).await;
    assert_eq!(started.len(), 2, "recorded with their subscription");

    let mut all = vec![CUST_SPOTIFY_ID, CUST_UBER_ID, CUST_COMODO_ID];
    all.sort();
    assert_eq!(subscribed(&env).await, all);
    for id in &started {
        env.get_subscription(*id).await.assert().is_active();
        assert!(
            env.get_invoices(*id).await.is_empty(),
            "a free plan writes no invoice"
        );
    }
    let origins = created_origins(&env).await;
    assert_eq!(
        origins
            .iter()
            .filter(|o| **o == Some(SubscriptionOrigin::SignupPlanBackfill))
            .count(),
        2,
        "{origins:?}"
    );
    assert_eq!(
        origins.iter().filter(|o| o.is_none()).count(),
        1,
        "Uber's own sign-up"
    );

    let again = processor
        .process_chunk(&job, &chunk(&job, 0, 2), Some(input))
        .await
        .expect("the chunk again");
    assert!(again.failures.is_empty());
    assert_eq!(subscribed(&env).await, all);
    assert_eq!(recorded(&env, &job).await.len(), 2, "none recorded twice");

    env.run_outbox_and_orchestration().await;
    for _ in 0..2 {
        env.advance(chrono::Duration::days(32));
        env.process_cycles().await;
        env.run_outbox_and_orchestration().await;
    }
    for id in &started {
        assert!(
            env.get_invoices(*id).await.is_empty(),
            "no invoice at its renewals"
        );
    }
    let subjects: Vec<_> = env
        .drain_email_queue()
        .await
        .into_iter()
        .map(|email| email.subject)
        .collect();
    assert!(subjects.is_empty(), "no customer email: {subjects:?}");
}

/// A customer whose subscription in the product line ended is left off, unless asked for.
#[tokio::test]
async fn churned_customers_are_left_off_unless_asked_for() {
    let env = test_env().await;
    signup_plan(&env, Decimal::ZERO, None).await;
    let offer = env
        .services()
        .signup_offer(TENANT_ID, Some(PRODUCT_FAMILY_ID), "EUR")
        .await
        .expect("the sign-up offer");
    let ended = env
        .services()
        .subscribe_to_signup_plan(Actor::System, TENANT_ID, CUST_UBER_ID, offer)
        .await
        .expect("Uber on the plan");
    {
        use diesel_models::schema::subscription::dsl as s;
        diesel::update(s::subscription.filter(s::id.eq(ended.id)))
            .set(s::status.eq(SubscriptionStatusEnum::Cancelled))
            .execute(&mut env.conn().await)
            .await
            .expect("Uber churned");
    }

    let preview = |include_churned| {
        env.services()
            .signup_backfill_preview(TENANT_ID, PRODUCT_FAMILY_ID, include_churned)
    };
    let customers = |include_churned| {
        env.services()
            .signup_backfill_customers(TENANT_ID, PRODUCT_FAMILY_ID, include_churned)
    };
    let without = preview(false).await.expect("a preview");
    let with = preview(true).await.expect("a preview with the churned");
    assert_eq!((without.churned, with.churned), (1, 1));
    assert_eq!(with.customers, without.customers + 1);
    assert!(
        !customers(false)
            .await
            .expect("ids")
            .1
            .contains(&CUST_UBER_ID)
    );
    assert!(
        customers(true)
            .await
            .expect("ids")
            .1
            .contains(&CUST_UBER_ID)
    );
}

/// The plan's eligibility rule leaves off the customers it does not let in: each is listed with
/// the reason, none is subscribed.
#[tokio::test]
async fn the_eligibility_rule_leaves_off_whom_it_does_not_let_in() {
    let env = test_env().await;
    let plan = signup_plan(&env, Decimal::ZERO, None).await;
    env.store()
        .set_plan_eligibility(
            Actor::System,
            TENANT_ID,
            plan,
            Eligibility {
                countries: vec!["PT".to_owned()],
                ..Default::default()
            },
        )
        .await
        .expect("a country rule");

    let preview = env
        .services()
        .signup_backfill_preview(TENANT_ID, PRODUCT_FAMILY_ID, false)
        .await
        .expect("a preview");
    assert!(preview.eligibility_rule);
    assert_eq!(
        (preview.customers, preview.not_eligible),
        (0, 3),
        "none of the seeded customers is in Portugal"
    );
    let (version, customers) = env
        .services()
        .signup_backfill_customers(TENANT_ID, PRODUCT_FAMILY_ID, false)
        .await
        .expect("the customers");
    let job = job(&env, version).await;
    let result = SignupPlanBackfillProcessor::new(env.services().clone())
        .process_chunk(
            &job,
            &chunk(&job, 0, customers.len() as i32),
            Some(input_of(&customers)),
        )
        .await
        .expect("the chunk");
    assert_eq!(result.failures.len(), customers.len(), "each one listed");
    assert!(
        result
            .failures
            .iter()
            .all(|failure| failure.reason.contains("eligibility rule")),
        "{:?}",
        result.failures
    );
    assert!(subscribed(&env).await.is_empty());
}

/// A sign-up plan that bills, after a trial or at once, cannot start existing customers.
#[tokio::test]
async fn only_a_free_signup_plan_starts_existing_customers() {
    let env = test_env().await;
    signup_plan(&env, dec!(10), Some(14)).await;

    let refused = env
        .services()
        .signup_backfill_preview(TENANT_ID, PRODUCT_FAMILY_ID, false)
        .await
        .expect_err("a plan that bills after its trial");
    assert!(
        format!("{refused:?}").contains("Only a free sign-up plan"),
        "{refused:?}"
    );
    let refused = env
        .services()
        .backfill_signup_plan(
            Actor::System,
            TENANT_ID,
            PRODUCT_FAMILY_ID,
            PlanVersionId::new(),
            None,
            CUST_SPOTIFY_ID,
        )
        .await
        .expect_err("a plan that bills after its trial");
    assert!(
        format!("{refused:?}").contains("Only a free sign-up plan"),
        "{refused:?}"
    );
    assert!(subscribed(&env).await.is_empty());
}

/// A plan including credit writes an invoice to grant it: it starts no existing customer.
#[tokio::test]
async fn a_plan_including_credit_starts_no_existing_customer() {
    let env = test_env().await;
    let (plan, version) = (PlanId::new(), PlanVersionId::new());
    let mut credits = SeedComp::rate(
        PriceComponentId::new(),
        "Included credits",
        PRODUCT_PLATFORM_FEE_ID,
        PriceId::new(),
        DieselBillingPeriodEnum::Monthly,
        Decimal::ZERO,
    )
    .with_structure(FeeStructure::Credit {});
    credits.prices[0].pricing = Pricing::Credit {
        unit_price: Decimal::ZERO,
        quantity: 1,
        credit_amount: dec!(500),
        expiry: None,
        credit_unit_id: None,
        rollover: None,
        once: false,
    };
    PlanSeed::new(plan, "Hobby with credits", version)
        .components(vec![credits])
        .seed(&mut env.conn().await)
        .await
        .expect("a plan including credit");
    env.store()
        .save_product_family_journey(
            TENANT_ID,
            ProductFamilyJourney {
                product_family_id: PRODUCT_FAMILY_ID,
                signup_plan_id: Some(plan),
                signup_option: None,
                period_switch: true,
                self_serve_downgrade: false,
            },
        )
        .await
        .expect("the sign-up plan");

    let refused = env
        .services()
        .signup_backfill_preview(TENANT_ID, PRODUCT_FAMILY_ID, false)
        .await
        .expect_err("a plan granting credit");
    assert!(
        format!("{refused:?}").contains("grants credit"),
        "{refused:?}"
    );
}

/// A free plan with a trial starts existing customers without it: they asked for none.
#[tokio::test]
async fn a_backfill_starts_no_trial() {
    let env = test_env().await;
    signup_plan(&env, Decimal::ZERO, Some(14)).await;
    let version = signup_version(&env).await;
    let BackfillOutcome::Subscribed(id) = backfill(&env, version, CUST_SPOTIFY_ID)
        .await
        .expect("started")
    else {
        panic!("Spotify has no plan");
    };
    env.get_subscription(id).await.assert().is_active();
    use diesel_models::schema::subscription_trial::dsl as t;
    let trials: i64 = t::subscription_trial
        .filter(t::subscription_id.eq(id))
        .count()
        .get_result(&mut env.conn().await)
        .await
        .expect("its trial records");
    assert_eq!(trials, 0, "no trial");
}

/// Once the sign-up plan changed, the customers previewed for the earlier one are not started.
#[tokio::test]
async fn a_changed_signup_plan_starts_no_one_previewed_for_another() {
    let env = test_env().await;
    signup_plan(&env, Decimal::ZERO, None).await;
    let previewed = signup_version(&env).await;
    signup_plan(&env, Decimal::ZERO, None).await;

    let refused = backfill(&env, previewed, CUST_SPOTIFY_ID)
        .await
        .expect_err("another plan now");
    assert!(format!("{refused:?}").contains("changed"), "{refused:?}");
    assert!(subscribed(&env).await.is_empty());
}

/// Two backfills reaching one customer at once start it once.
#[tokio::test]
async fn concurrent_backfills_start_a_customer_once() {
    let env = test_env().await;
    signup_plan(&env, Decimal::ZERO, None).await;
    let version = signup_version(&env).await;

    let (first, second) = tokio::join!(
        backfill(&env, version, CUST_SPOTIFY_ID),
        backfill(&env, version, CUST_SPOTIFY_ID)
    );
    let mut outcomes = [first.expect("first"), second.expect("second")]
        .map(|outcome| matches!(outcome, BackfillOutcome::Subscribed(_)));
    outcomes.sort();
    assert_eq!(outcomes, [false, true], "one subscribed, one skipped");
    assert_eq!(subscribed(&env).await, vec![CUST_SPOTIFY_ID]);
}

/// The generic batch job endpoint refuses a backfill: only the customer journey starts one, as
/// it checks the permission, the entitlement and who is started.
#[tokio::test]
async fn the_batch_job_endpoint_refuses_a_backfill() {
    use meteroid_grpc::meteroid::api::batchjobs::v1::{BatchJobType, CreateBatchJobRequest};
    use meteroid_store::domain::PaginationRequest;

    let env = test_env().await;
    signup_plan(&env, Decimal::ZERO, None).await;
    let version = signup_version(&env).await;
    let auth = crate::meteroid_it::svc_auth::login(env.setup.channel.clone()).await;
    let clients = crate::meteroid_it::clients::AllClients::from_channel(
        env.setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let refused = clients
        .batch_jobs
        .clone()
        .create_batch_job(CreateBatchJobRequest {
            file_data: Some(input_of(&[CUST_SPOTIFY_ID]).to_vec()),
            job_type: BatchJobType::SignupPlanBackfill as i32,
            params: [
                (
                    "product_family_id".to_owned(),
                    PRODUCT_FAMILY_ID.to_string(),
                ),
                ("plan_version_id".to_owned(), version.to_string()),
            ]
            .into(),
            file_name: None,
            force_duplicate: false,
        })
        .await
        .expect_err("refused");
    assert_eq!(refused.code(), tonic::Code::InvalidArgument, "{refused:?}");
    let jobs = env
        .store()
        .list_batch_jobs(
            TENANT_ID,
            None,
            &[],
            None,
            PaginationRequest {
                per_page: Some(10),
                page: 0,
            },
        )
        .await
        .expect("jobs");
    assert!(jobs.items.is_empty(), "no job created");
}

/// A plan reading usage, or one with a minimum commitment, may write a document: it starts no
/// existing customer.
#[tokio::test]
async fn a_plan_reading_usage_or_with_a_minimum_starts_no_existing_customer() {
    use meteroid_store::domain::UsagePricingModel;
    use meteroid_store::domain::plan_minimums::MinimumScope;
    use meteroid_store::repositories::PlansInterface;

    async fn refused(env: &TestEnv) {
        let refused = env
            .services()
            .signup_backfill_preview(TENANT_ID, PRODUCT_FAMILY_ID, false)
            .await
            .expect_err("refused");
        assert!(
            format!("{refused:?}").contains("Only a free sign-up plan"),
            "{refused:?}"
        );
    }

    let env = test_env().await;

    let (plan, version) = (PlanId::new(), PlanVersionId::new());
    PlanSeed::new(plan, "Metered at nothing", version)
        .components(vec![SeedComp::usage(
            PriceComponentId::new(),
            "Bandwidth",
            PRODUCT_BANDWIDTH_ID,
            METRIC_BANDWIDTH,
            PriceId::new(),
            DieselBillingPeriodEnum::Monthly,
            UsagePricingModel::PerUnit {
                rate: Decimal::ZERO,
            },
        )])
        .seed(&mut env.conn().await)
        .await
        .expect("a plan metered at nothing");
    env.store()
        .save_product_family_journey(
            TENANT_ID,
            ProductFamilyJourney {
                product_family_id: PRODUCT_FAMILY_ID,
                signup_plan_id: Some(plan),
                signup_option: None,
                period_switch: true,
                self_serve_downgrade: false,
            },
        )
        .await
        .expect("the sign-up plan");
    refused(&env).await;

    signup_plan(&env, Decimal::ZERO, None).await;
    let version = signup_version(&env).await;
    crate::helpers::plans::as_draft(
        env.store(),
        version,
        env.store()
            .set_plan_minimum(version, TENANT_ID, 1_000, MinimumScope::AllComponents),
    )
    .await
    .expect("a minimum commitment");
    refused(&env).await;
}

/// The REST API shows no backfill: its job type is the customer journey's, outside the API's
/// contract.
#[tokio::test]
async fn the_rest_api_shows_no_backfill_job() {
    let fixture = crate::test_rest_wallets::fixture().await;
    let create = |job_type| {
        fixture.setup.store.create_batch_job(BatchJobNew {
            tenant_id: TENANT_ID,
            job_type,
            input_source_key: None,
            input_params: Some(json!({})),
            file_hash: None,
            created_by: USER_ID.as_uuid(),
            input_file_name: None,
        })
    };
    let backfill = create(BatchJobTypeEnum::SignupPlanBackfill)
        .await
        .expect("a backfill job");
    let import = create(BatchJobTypeEnum::CustomerCsvImport)
        .await
        .expect("an import job");

    let (status, listed) = fixture.get("/api/v1/batch-jobs").await;
    assert_eq!(status, reqwest::StatusCode::OK, "{listed}");
    let ids: Vec<_> = listed["data"]
        .as_array()
        .expect("jobs")
        .iter()
        .map(|job| job["id"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert_eq!(ids, vec![import.id.to_string()], "{listed}");
    let (status, _) = fixture
        .get(&format!("/api/v1/batch-jobs/{}", backfill.id))
        .await;
    assert_eq!(status, reqwest::StatusCode::NOT_FOUND);
}

/// The sign-up plans a customer created by hand can start on: a free one, and one with a trial.
#[tokio::test]
async fn the_signup_plans_a_customer_can_start_on_are_listed() {
    let env = test_env().await;
    assert!(
        env.services()
            .list_signup_plans(TENANT_ID)
            .await
            .expect("none yet")
            .is_empty()
    );

    signup_plan(&env, Decimal::ZERO, None).await;
    let plans = env
        .services()
        .list_signup_plans(TENANT_ID)
        .await
        .expect("plans");
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].product_family_id, PRODUCT_FAMILY_ID);
    assert_eq!(plans[0].plan_name, "Hobby");
    assert!(plans[0].free);

    signup_plan(&env, dec!(10), Some(14)).await;
    let plans = env
        .services()
        .list_signup_plans(TENANT_ID)
        .await
        .expect("plans");
    assert_eq!(plans.len(), 1);
    assert!(!plans[0].free, "billed after its trial");
}
