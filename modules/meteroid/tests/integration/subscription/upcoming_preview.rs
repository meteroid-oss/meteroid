//! The next-invoice preview bills what the lifecycle will: a free trial's usage is never billed,
//! and a final invoice is dated, titled and due as it will be issued.
use chrono::{Days, NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::ids::SubscriptionId;
use meteroid_grpc::meteroid::api::subscriptions::v1 as api;
use meteroid_grpc::meteroid::portal::subscription::v1 as portal;
use meteroid_store::domain::InvoicingEntityPatch;
use meteroid_store::domain::{
    CreateSubscription, CreateSubscriptionComponents, SubscriptionActivationCondition,
    SubscriptionNew, SubscriptionStatusEnum,
};
use meteroid_store::repositories::invoicing_entities::InvoicingEntityInterface;
use meteroid_store::repositories::subscriptions::{
    CancellationEffectiveAt, SubscriptionInterfaceAuto,
};
use rust_decimal_macros::dec;

use crate::data::ids::{
    COMP_LEETCODE_RATE_ID, CUST_UBER_ID, INVOICING_ENTITY_ID, METRIC_BANDWIDTH,
    PLAN_VERSION_1_LEETCODE_ID, PLAN_VERSION_USAGE_ALPHA_ID, PLAN_VERSION_USAGE_BETA_ID, TENANT_ID,
    USER_ID,
};
use crate::helpers::credit_recompute::{fixture, metric_component};
use crate::meteroid_it::container::MeteroidSetup;
use meteroid_store::domain::UsagePricingModel;

/// The customer on usage at €1.00 a unit only (the plan's fee removed).
fn usage_request(start: NaiveDate, trial_duration: Option<u32>) -> CreateSubscription {
    CreateSubscription {
        subscription: SubscriptionNew {
            customer_id: CUST_UBER_ID,
            plan_version_id: PLAN_VERSION_1_LEETCODE_ID,
            net_terms: Some(30),
            invoice_memo: None,
            invoice_threshold: None,
            start_date: start,
            end_date: None,
            billing_start_date: None,
            activation_condition: SubscriptionActivationCondition::OnStart,
            trial_duration,
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
        price_components: Some(CreateSubscriptionComponents {
            parameterized_components: vec![],
            overridden_components: vec![],
            extra_components: vec![metric_component(
                "Bandwidth",
                METRIC_BANDWIDTH,
                UsagePricingModel::PerUnit { rate: dec!(1) },
            )],
            remove_components: vec![COMP_LEETCODE_RATE_ID],
        }),
        add_ons: None,
        coupons: None,
        entitlements: vec![],
        minimum_override: None,
        drop_minimum: false,
    }
}

async fn subscribe(
    setup: &MeteroidSetup,
    start: NaiveDate,
    trial_duration: Option<u32>,
) -> SubscriptionId {
    setup
        .services
        .insert_subscription(
            Actor::System,
            usage_request(start, trial_duration),
            TENANT_ID,
        )
        .await
        .expect("a subscription")
        .id
}

/// RF-37: creating an unfiltered subscription next to an unfiltered one pricing the same metric
/// is flagged by the server; a usage filter on the new one clears it.
#[tokio::test]
async fn an_unfiltered_sibling_sharing_a_metric_is_flagged_at_create() {
    let f = fixture().await;
    let today = Utc::now().date_naive();
    let sibling = subscribe(&f.setup, today - Days::new(3), None).await;

    let unfiltered = usage_request(today, None);
    let (_, details) = f
        .setup
        .services
        .preview_create_subscription(&unfiltered, TENANT_ID)
        .await
        .expect("preview");
    assert_eq!(
        f.setup
            .services
            .unfiltered_overlaps(TENANT_ID, &details)
            .await
            .expect("overlaps"),
        vec![(sibling, vec![METRIC_BANDWIDTH])]
    );

    let mut filtered = usage_request(today, None);
    filtered.subscription.usage_filter = Some(
        meteroid_store::domain::UsageFilter::new("team".into(), "mobile".into()).expect("filter"),
    );
    let (_, details) = f
        .setup
        .services
        .preview_create_subscription(&filtered, TENANT_ID)
        .await
        .expect("preview");
    assert!(
        f.setup
            .services
            .unfiltered_overlaps(TENANT_ID, &details)
            .await
            .expect("overlaps")
            .is_empty()
    );
}

/// MT-16: the invoice ending a free trial opens cycle 0, with no arrears; its preview bills none.
#[tokio::test]
async fn the_preview_at_a_free_trials_end_bills_no_trial_usage() {
    let f = fixture().await;
    let sub_id = subscribe(&f.setup, Utc::now().date_naive() - Days::new(3), Some(14)).await;
    let details = f
        .setup
        .store
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("details");
    assert_eq!(
        details.subscription.status,
        SubscriptionStatusEnum::TrialActive
    );

    let forecast = f
        .setup
        .services
        .upcoming_invoice_forecast(&details)
        .await
        .expect("forecast");
    assert!(
        forecast.content.invoice_lines.is_empty(),
        "the trial's usage is not billed: {:?}",
        forecast.content.invoice_lines
    );
    assert_eq!(forecast.content.total, 0);
    let content = f
        .setup
        .services
        .compute_upcoming_invoice(&details)
        .await
        .expect("upcoming invoice");
    assert_eq!(content.total, 0, "every next-invoice preview agrees");
}

/// MT-16, after a plan change scheduled for the trial's end: the change leaves the EndTrial
/// transition, so the preview still opens cycle 0 and bills none of the trial's usage.
#[tokio::test]
async fn a_plan_change_at_a_free_trials_end_previews_no_trial_usage() {
    let f = fixture().await;
    let sub_id = crate::harness::subscription()
        .customer(CUST_UBER_ID)
        .plan_version(PLAN_VERSION_USAGE_ALPHA_ID)
        .start_date(Utc::now().date_naive() - Days::new(3))
        .on_start()
        .trial_days(14)
        .create(&f.setup.services)
        .await;
    f.setup
        .services
        .schedule_plan_change(
            Actor::System,
            sub_id,
            TENANT_ID,
            PLAN_VERSION_USAGE_BETA_ID,
            vec![],
        )
        .await
        .expect("plan change at the trial's end");
    let details = f
        .setup
        .store
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("details");
    assert_eq!(
        details.subscription.status,
        SubscriptionStatusEnum::TrialActive
    );

    let forecast = f
        .setup
        .services
        .upcoming_invoice_forecast(&details)
        .await
        .expect("forecast");
    assert!(matches!(
        forecast.change,
        Some(meteroid_store::domain::credit_outlook::UpcomingChange::PlanChange { .. })
    ));
    let trial_usage: i64 = forecast
        .content
        .invoice_lines
        .iter()
        .filter(|line| line.metric_id == Some(METRIC_BANDWIDTH))
        .map(|line| line.amount_subtotal)
        .sum();
    assert_eq!(trial_usage, 0, "{:?}", forecast.content.invoice_lines);
}

/// A scheduled pause previews the last invoice before it, titled as a pause, not a final invoice.
#[tokio::test]
async fn a_scheduled_pause_previews_the_last_invoice_before_it() {
    use meteroid_store::domain::scheduled_events::{ScheduledEventData, ScheduledEventNew};
    use meteroid_store::repositories::SubscriptionInterface;

    let f = fixture().await;
    let sub_id = subscribe(&f.setup, Utc::now().date_naive() - Days::new(3), None).await;
    let details = f
        .setup
        .store
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("details");
    let pause_on = details.subscription.current_period_end.expect("period end") - Days::new(5);
    let mut conn = f.setup.store.get_conn().await.expect("connection");
    f.setup
        .store
        .schedule_events(
            &mut conn,
            vec![ScheduledEventNew {
                subscription_id: sub_id,
                tenant_id: TENANT_ID,
                scheduled_time: pause_on.and_hms_opt(0, 0, 0).unwrap(),
                event_data: ScheduledEventData::PauseSubscription,
                source: "test".to_string(),
                created_by: Actor::System,
            }],
        )
        .await
        .expect("pause");
    drop(conn);

    let auth = crate::meteroid_it::svc_auth::login(f.setup.channel.clone()).await;
    let invoice = crate::meteroid_it::clients::AllClients::from_channel(
        f.setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    )
    .subscriptions
    .get_upcoming_invoice(api::GetUpcomingInvoiceRequest {
        subscription_id: sub_id.to_string(),
    })
    .await
    .expect("upcoming invoice")
    .into_inner()
    .invoice
    .expect("invoice");
    assert_eq!(invoice.change, api::UpcomingInvoiceChange::Pause as i32);
    assert_eq!(invoice.invoice_date, pause_on.to_string());
    assert!(!invoice.not_invoiced);
}

/// RF-33, RF-36: a cancellation before the period's end is the final invoice, on that date. It
/// is issued (and charged) when its grace period ends, and due net terms after that issue day,
/// as finalization re-dates it; the dashboard and the portal agree.
#[tokio::test]
async fn a_scheduled_cancellation_previews_the_final_invoice_on_its_date() {
    final_invoice_dates(12, 0).await;
}

#[tokio::test]
async fn a_full_day_of_grace_issues_and_dues_the_final_invoice_a_day_later() {
    final_invoice_dates(24, 1).await;
}

async fn final_invoice_dates(grace_period_hours: i32, issued_days_later: u64) {
    let f = fixture().await;
    f.setup
        .store
        .patch_invoicing_entity(
            Actor::User { id: USER_ID },
            InvoicingEntityPatch {
                id: INVOICING_ENTITY_ID,
                consolidate_recurring_invoices: Some(false),
                grace_period_hours: Some(grace_period_hours),
                ..Default::default()
            },
            TENANT_ID,
        )
        .await
        .expect("grace period");
    let sub_id = subscribe(&f.setup, Utc::now().date_naive() - Days::new(3), None).await;
    let before = f
        .setup
        .store
        .get_subscription_details(TENANT_ID, sub_id)
        .await
        .expect("details");
    let period_end = before.subscription.current_period_end.expect("period end");
    let cancel_on = period_end - Days::new(5);
    f.setup
        .services
        .cancel_subscription(
            Actor::System,
            sub_id,
            TENANT_ID,
            None,
            CancellationEffectiveAt::Date(cancel_on),
        )
        .await
        .expect("cancel");
    let charge_on = cancel_on + Days::new(issued_days_later);
    let due_on = charge_on + Days::new(30);

    let auth = crate::meteroid_it::svc_auth::login(f.setup.channel.clone()).await;
    let clients = crate::meteroid_it::clients::AllClients::from_channel(
        f.setup.channel.clone(),
        auth.token.as_str(),
        "TESTORG",
        "testslug",
    );
    let invoice = clients
        .subscriptions
        .clone()
        .get_upcoming_invoice(api::GetUpcomingInvoiceRequest {
            subscription_id: sub_id.to_string(),
        })
        .await
        .expect("upcoming invoice")
        .into_inner()
        .invoice
        .expect("invoice");
    assert_eq!(invoice.change, api::UpcomingInvoiceChange::Ends as i32);
    assert_eq!(invoice.invoice_date, cancel_on.to_string());
    assert_eq!(invoice.due_date, Some(due_on.to_string()));
    assert_eq!(invoice.charge_date, charge_on.to_string());
    assert_eq!(invoice.new_plan_name, None);

    let token = meteroid_store::jwt_claims::generate_portal_token(
        &f.setup.config.jwt_secret,
        TENANT_ID,
        meteroid_store::jwt_claims::ResourceAccess::Customer(CUST_UBER_ID),
        None,
        None,
        false,
    )
    .expect("portal token");
    let mut request = tonic::Request::new(portal::GetUpcomingInvoiceRequest {
        subscription_id: sub_id.to_string(),
    });
    request
        .metadata_mut()
        .insert("x-portal-token", token.parse().expect("token header"));
    let portal_invoice =
        portal::portal_subscription_service_client::PortalSubscriptionServiceClient::new(
            f.setup.channel.clone(),
        )
        .get_upcoming_invoice(request)
        .await
        .expect("portal upcoming invoice")
        .into_inner()
        .invoice
        .expect("invoice");
    assert_eq!(portal_invoice.invoice_date, cancel_on.to_string());
    assert_eq!(portal_invoice.charge_date, charge_on.to_string());
    assert_eq!(portal_invoice.due_date, due_on.to_string());
}
