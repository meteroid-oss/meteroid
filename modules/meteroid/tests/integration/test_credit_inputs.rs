//! Live-credit input tokens and their change feed (live-recompute v2, step 1): who bumps what,
//! once per transaction, and a feed whose poll never passes a transaction still open.
//!
//! Tests run in parallel on one Postgres cluster, whose `xmin` any of them can hold back. Writer
//! assertions therefore read `credit_input_change` directly; poll tests first wait until the
//! cluster's `xmin` has passed the rows they expect.
use crate::data::ids::{
    CUST_SPOTIFY_ID, CUST_UBER_CONNECTION_ID, CUST_UBER_ID, METRIC_DATABASE_SIZE,
    PLAN_VERSION_1_LEETCODE_ID, PLAN_VERSION_NOTION_ID, PRODUCT_FAMILY_ID, TENANT_ID,
};
use crate::harness::{subscription, test_env_with_seed};
use crate::meteroid_it::container::{MeteroidSetup, SeedLevel};
use chrono::{Days, NaiveDate, Utc};
use common_domain::actor::Actor;
use common_domain::ids::{
    AliasOr, BaseId, CustomerId, CustomerPaymentMethodId, EntitlementEntityId, ProductId, TenantId,
};
use diesel::{QueryableByName, sql_query, sql_types};
use diesel_async::{AsyncConnection, RunQueryDsl, SimpleAsyncConnection};
use diesel_models::PgConn;
use diesel_models::credit_inputs::{CreditFeedCursor, CreditInputChangeRow};
use diesel_models::customer_credit::CustomerCreditAccountRow;
use diesel_models::customers::CustomerRow;
use diesel_models::enums::{CreditInputChangeKind, FeeTypeEnum};
use diesel_models::errors::{DatabaseError, DatabaseErrorContainer};
use diesel_models::products::ProductRowNew;
use diesel_models::query::credit_inputs;
use diesel_models::schema::sql_types::CreditInputChangeKindEnum;
use meteroid_store::domain::entitlements::{
    EntitlementNew, EntitlementValue, FeatureNew, FeatureType,
};
use meteroid_store::domain::prices::{FeeStructure, MatrixPriceUpdate, UsageModel};
use meteroid_store::domain::{CustomerPaymentMethodNew, PaymentMethodTypeEnum, SubscriptionPatch};
use meteroid_store::repositories::billable_metrics::BillableMetricInterface;
use meteroid_store::repositories::credit::CustomerCreditWalletsInterface;
use meteroid_store::repositories::credit::inputs::{FEED_DAYS_AHEAD, FEED_RETENTION_DAYS};
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::price_components::PriceComponentInterface;
use meteroid_store::repositories::price_components::PriceComponentInterfaceAuto;
use meteroid_store::repositories::subscriptions::CancellationEffectiveAt;
use meteroid_store::repositories::{
    CustomersInterface, EntitlementsInterface, InvoiceInterface, PlansInterface,
    SubscriptionInterface,
};
use scoped_futures::ScopedFutureExt;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::Barrier;
use uuid::Uuid;

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

#[derive(QueryableByName, Debug, Clone)]
struct FeedRow {
    #[diesel(sql_type = sql_types::BigInt)]
    id: i64,
    #[diesel(sql_type = sql_types::BigInt)]
    store_xid: i64,
    #[diesel(sql_type = sql_types::Uuid)]
    tenant_id: Uuid,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Uuid>)]
    customer_id: Option<Uuid>,
    #[diesel(sql_type = CreditInputChangeKindEnum)]
    kind: CreditInputChangeKind,
}

#[derive(QueryableByName)]
struct Scalar {
    #[diesel(sql_type = sql_types::BigInt)]
    value: i64,
}

/// The last feed row id: rows written afterwards have larger ids.
async fn feed_mark(conn: &mut PgConn) -> TestResult<i64> {
    let row: Scalar = sql_query("SELECT COALESCE(max(id), 0) AS value FROM credit_input_change")
        .get_result(conn)
        .await
        .map_err(test_error)?;
    Ok(row.value)
}

async fn feed_since(conn: &mut PgConn, mark: i64) -> TestResult<Vec<FeedRow>> {
    sql_query(
        "SELECT id, store_xid::text::bigint AS store_xid, tenant_id, customer_id, kind
         FROM credit_input_change WHERE id > $1 ORDER BY id",
    )
    .bind::<sql_types::BigInt, _>(mark)
    .get_results(conn)
    .await
    .map_err(test_error)
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Subject {
    Customer(CustomerId),
    Tenant(TenantId),
}

impl Subject {
    fn matches(&self, row: &FeedRow) -> bool {
        match self {
            Subject::Customer(customer) => row.customer_id == Some(customer.as_uuid()),
            Subject::Tenant(tenant) => {
                row.customer_id.is_none() && row.tenant_id == tenant.as_uuid()
            }
        }
    }
}

/// The subject has rows of `kind` since `mark`, at most one per transaction.
fn assert_recorded(rows: &[FeedRow], subject: Subject, kind: CreditInputChangeKind, name: &str) {
    let mine: Vec<&FeedRow> = rows
        .iter()
        .filter(|row| subject.matches(row) && row.kind == kind)
        .collect();
    assert!(
        !mine.is_empty(),
        "{name}: no {kind:?} row for {subject:?} in {rows:?}"
    );
    let mut xids: Vec<i64> = mine.iter().map(|row| row.store_xid).collect();
    xids.sort_unstable();
    let before = xids.len();
    xids.dedup();
    assert_eq!(
        before,
        xids.len(),
        "{name}: one {kind:?} row per transaction"
    );
}

async fn token(conn: &mut PgConn, subject: Subject) -> TestResult<Option<i64>> {
    match subject {
        Subject::Customer(customer) => credit_inputs::customer_token(conn, TENANT_ID, customer)
            .await
            .map_err(test_error),
        Subject::Tenant(tenant) => credit_inputs::tenant_token(conn, tenant)
            .await
            .map_err(test_error),
    }
}

/// Runs `write` and checks it moved the subject's token and wrote its feed row once per
/// transaction.
async fn expect_bump<F, T>(
    conn: &mut PgConn,
    name: &str,
    subject: Subject,
    write: F,
) -> TestResult<T>
where
    F: Future<Output = TestResult<T>>,
{
    let mark = feed_mark(conn).await?;
    let before = token(conn, subject).await?;
    let written = write.await?;
    let after = token(conn, subject).await?;
    assert!(
        after.is_some() && after != before,
        "{name}: the token moves"
    );
    let kind = match subject {
        Subject::Customer(_) => CreditInputChangeKind::Inputs,
        Subject::Tenant(_) => CreditInputChangeKind::Tenant,
    };
    assert_recorded(&feed_since(conn, mark).await?, subject, kind, name);
    Ok(written)
}

#[tokio::test]
async fn customer_and_tenant_writers_bump_their_tokens() -> TestResult {
    let env = test_env_with_seed(SeedLevel::PLANS).await;
    env.seed_payments().await;
    let mut conn = env.conn().await;
    let uber = Subject::Customer(CUST_UBER_ID);
    let tenant = Subject::Tenant(TENANT_ID);

    let subscription_id = expect_bump(&mut conn, "subscription create", uber, async {
        Ok(subscription()
            .customer(CUST_UBER_ID)
            .plan_version(PLAN_VERSION_1_LEETCODE_ID)
            .start_date(Utc::now().date_naive())
            .on_start()
            .no_trial()
            .create(env.services())
            .await)
    })
    .await?;

    expect_bump(&mut conn, "subscription patch", uber, async {
        env.store()
            .patch_subscription(
                TENANT_ID,
                SubscriptionPatch {
                    id: subscription_id,
                    charge_automatically: None,
                    auto_advance_invoices: None,
                    net_terms: None,
                    invoice_memo: Some(Some("memo".to_owned())),
                    purchase_order: None,
                    payment_methods_config: None,
                    custom_properties: None,
                    usage_filter: None,
                },
                Actor::System,
            )
            .await
            .map(|_| ())
            .map_err(test_error)
    })
    .await?;

    let draft = env
        .insert_draft_invoice(subscription_id, Utc::now().date_naive() + Days::new(40))
        .await;
    expect_bump(&mut conn, "draft delete", uber, async {
        env.store()
            .delete_invoice(Actor::System, draft.id, TENANT_ID)
            .await
            .map_err(test_error)
    })
    .await?;

    expect_bump(&mut conn, "subscription cancel", uber, async {
        env.services()
            .cancel_subscription(
                Actor::System,
                subscription_id,
                TENANT_ID,
                None,
                CancellationEffectiveAt::EndOfBillingPeriod,
            )
            .await
            .map(|_| ())
            .map_err(test_error)
    })
    .await?;

    let feature = env
        .store()
        .create_feature(
            Actor::System,
            FeatureNew {
                tenant_id: TENANT_ID,
                product_id: None,
                name: "Exports".to_owned(),
                code: "exports".to_owned(),
                description: None,
                feature_type: FeatureType::Boolean,
                entitlement: None,
            },
        )
        .await
        .map_err(test_error)?;
    expect_bump(&mut conn, "customer entitlement", uber, async {
        env.store()
            .create_entitlement(EntitlementNew {
                tenant_id: TENANT_ID,
                feature_id: feature.id,
                entity: EntitlementEntityId::Customer(CUST_UBER_ID),
                value: EntitlementValue::Boolean { enabled: true },
            })
            .await
            .map(|_| ())
            .map_err(test_error)
    })
    .await?;

    expect_bump(&mut conn, "payment method", uber, async {
        env.store()
            .upsert_payment_method(CustomerPaymentMethodNew {
                id: CustomerPaymentMethodId::new(),
                tenant_id: TENANT_ID,
                customer_id: CUST_UBER_ID,
                connection_id: CUST_UBER_CONNECTION_ID,
                external_payment_method_id: "pm_credit_inputs".to_owned(),
                payment_method_type: PaymentMethodTypeEnum::Card,
                account_number_hint: None,
                card_brand: Some("visa".to_owned()),
                card_last4: Some("4242".to_owned()),
                card_exp_month: Some(12),
                card_exp_year: Some(2031),
                fingerprint: None,
                saved_for_payment: false,
            })
            .await
            .map(|_| ())
            .map_err(test_error)
    })
    .await?;

    let draft_version = env
        .store()
        .copy_plan_version_to_draft(PLAN_VERSION_1_LEETCODE_ID, TENANT_ID)
        .await
        .map_err(test_error)?;
    expect_bump(&mut conn, "plan publish", tenant, async {
        env.store()
            .publish_plan_version(Actor::System, draft_version.id, TENANT_ID)
            .await
            .map(|_| ())
            .map_err(test_error)
    })
    .await?;

    let matrix = ProductId::new();
    ProductRowNew {
        id: matrix,
        name: "Matrix".to_owned(),
        description: None,
        tenant_id: TENANT_ID,
        product_family_id: PRODUCT_FAMILY_ID,
        fee_type: FeeTypeEnum::Usage,
        fee_structure: serde_json::to_value(FeeStructure::Usage {
            metric_id: METRIC_DATABASE_SIZE,
            model: UsageModel::Matrix,
        })
        .map_err(test_error)?,
        catalog: true,
        tax_category_id: None,
    }
    .insert(&mut conn)
    .await
    .map_err(test_error)?;
    expect_bump(&mut conn, "matrix prices", tenant, async {
        env.services()
            .update_matrix_prices(
                TENANT_ID,
                matrix,
                MatrixPriceUpdate {
                    add_rows: vec![],
                    remove_rows: vec![],
                },
                Uuid::nil(),
            )
            .await
            .map(|_| ())
            .map_err(test_error)
    })
    .await?;

    // Only a draft's components can be removed.
    let draft = env
        .store()
        .copy_plan_version_to_draft(PLAN_VERSION_NOTION_ID, TENANT_ID)
        .await
        .map_err(test_error)?;
    let component = env
        .store()
        .list_price_components(draft.id, TENANT_ID)
        .await
        .map_err(test_error)?
        .into_iter()
        .find(|component| component.name == "Seats")
        .ok_or("the draft's seats")?;
    expect_bump(&mut conn, "price component delete", tenant, async {
        env.store()
            .delete_price_component(component.id, TENANT_ID)
            .await
            .map_err(test_error)
    })
    .await?;

    expect_bump(&mut conn, "metric unarchive", tenant, async {
        env.store()
            .unarchive_billable_metric(METRIC_DATABASE_SIZE, TENANT_ID)
            .await
            .map_err(test_error)
    })
    .await?;

    Ok(())
}

#[tokio::test]
async fn invoice_and_credit_note_writers_bump_the_customer() -> TestResult {
    let (setup, services, store, invoice) = crate::test_credit_note::setup_unpaid_invoice().await;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let customer = Subject::Customer(invoice.customer_id);

    expect_bump(&mut conn, "credit note finalize", customer, async {
        crate::test_credit_note::create_and_finalize_debt_cancellation_cn(&store, &invoice).await;
        Ok(())
    })
    .await?;
    let corrected = expect_bump(&mut conn, "corrected invoice", customer, async {
        services
            .create_corrected_invoice_from(TENANT_ID, invoice.id)
            .await
            .map_err(test_error)
    })
    .await?;
    expect_bump(&mut conn, "invoice finalize", customer, async {
        services
            .finalize_invoice(Actor::System, corrected.id, TENANT_ID)
            .await
            .map(|_| ())
            .map_err(test_error)
    })
    .await?;
    expect_bump(&mut conn, "invoice void", customer, async {
        store
            .void_invoice(Actor::System, corrected.id, TENANT_ID)
            .await
            .map(|_| ())
            .map_err(test_error)
    })
    .await?;
    Ok(())
}

#[tokio::test]
async fn customer_writers_without_a_seeded_flow_bump_once() -> TestResult {
    let setup = setup().await;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let uber = Subject::Customer(CUST_UBER_ID);
    expect_bump(&mut conn, "archive", uber, async {
        setup
            .store
            .archive_customer(Actor::System, TENANT_ID, AliasOr::Id(CUST_UBER_ID))
            .await
            .map_err(test_error)
    })
    .await?;
    expect_bump(&mut conn, "unarchive", uber, async {
        setup
            .store
            .unarchive_customer(Actor::System, TENANT_ID, AliasOr::Id(CUST_UBER_ID))
            .await
            .map_err(test_error)
    })
    .await?;
    expect_bump(&mut conn, "first wallet", uber, async {
        setup
            .store
            .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "EUR")
            .await
            .map(|_| ())
            .map_err(test_error)
    })
    .await?;
    Ok(())
}

/// A second tenant of the seeded organization, copied from the first.
async fn second_tenant(conn: &mut PgConn) -> TestResult<TenantId> {
    let id = TenantId::new();
    conn.batch_execute(&format!(
        "CREATE TEMP TABLE tenant_copy AS SELECT * FROM tenant WHERE id = '{}';
         UPDATE tenant_copy SET id = '{}', slug = slug || '-inputs', name = name || ' inputs',
                                environment = 'DEVELOPMENT';
         INSERT INTO tenant SELECT * FROM tenant_copy;
         DROP TABLE tenant_copy;",
        TENANT_ID.as_uuid(),
        id.as_uuid()
    ))
    .await
    .map_err(test_error)?;
    Ok(id)
}

#[tokio::test]
async fn two_tenants_bumped_in_one_transaction_each_get_their_row() -> TestResult {
    let setup = setup().await;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let other = second_tenant(&mut conn).await?;
    let mark = feed_mark(&mut conn).await?;

    setup
        .store
        .transaction_bumping_tenants(|_| {
            async move { Ok(((), vec![other, TENANT_ID, other])) }.scope_boxed()
        })
        .await
        .map_err(test_error)?;

    let rows = feed_since(&mut conn, mark).await?;
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows[0].store_xid, rows[1].store_xid);
    for tenant in [TENANT_ID, other] {
        assert_recorded(
            &rows,
            Subject::Tenant(tenant),
            CreditInputChangeKind::Tenant,
            "tenant",
        );
        assert!(token(&mut conn, Subject::Tenant(tenant)).await?.is_some());
    }
    Ok(())
}

#[tokio::test]
async fn the_tenant_bump_is_its_transactions_last_row() -> TestResult {
    let setup = setup().await;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let mark = feed_mark(&mut conn).await?;

    setup
        .store
        .transaction_bumping_tenant(TENANT_ID, |conn| {
            async move {
                let lock = CustomerRow::lock(conn, TENANT_ID, CUST_UBER_ID).await?;
                credit_inputs::bump_customer(conn, &lock).await?;
                credit_inputs::record_ledger_change(conn, &lock).await?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
        .map_err(test_error)?;

    let rows = feed_since(&mut conn, mark).await?;
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(rows.iter().all(|row| row.store_xid == rows[0].store_xid));
    let last = rows.last().ok_or("rows")?;
    assert_eq!(Some(last.id), rows.iter().map(|row| row.id).max());
    assert_eq!(last.kind, CreditInputChangeKind::Tenant);
    assert!(last.customer_id.is_none());
    Ok(())
}

#[tokio::test]
async fn a_rolled_back_transaction_leaves_tokens_and_feed_unchanged() -> TestResult {
    let setup = setup().await;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let mark = feed_mark(&mut conn).await?;
    let customer_before = token(&mut conn, Subject::Customer(CUST_UBER_ID)).await?;
    let tenant_before = token(&mut conn, Subject::Tenant(TENANT_ID)).await?;

    let outcome = conn
        .transaction::<(), DatabaseErrorContainer, _>(async |conn| {
            let lock = CustomerRow::lock(conn, TENANT_ID, CUST_UBER_ID).await?;
            credit_inputs::bump_customer(conn, &lock).await?;
            credit_inputs::record_ledger_change(conn, &lock).await?;
            credit_inputs::bump_tenant(conn, TENANT_ID).await?;
            Err(DatabaseError::Others("rolled back on purpose".to_owned()).into())
        })
        .await;
    assert!(outcome.is_err());

    assert_eq!(
        token(&mut conn, Subject::Customer(CUST_UBER_ID)).await?,
        customer_before
    );
    assert_eq!(
        token(&mut conn, Subject::Tenant(TENANT_ID)).await?,
        tenant_before
    );
    assert!(feed_since(&mut conn, mark).await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn a_transaction_writes_one_row_per_customer_and_kind() -> TestResult {
    let setup = setup().await;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    setup
        .store
        .create_customer_credit_wallet(TENANT_ID, CUST_UBER_ID, "EUR")
        .await
        .map_err(test_error)?;
    let mark = feed_mark(&mut conn).await?;

    conn.transaction::<(), DatabaseErrorContainer, _>(async |conn| {
        let uber = CustomerRow::lock(conn, TENANT_ID, CUST_UBER_ID).await?;
        let spotify = CustomerRow::lock(conn, TENANT_ID, CUST_SPOTIFY_ID).await?;
        // Two ledger movements, as two entries of one payment would.
        CustomerCreditAccountRow::bump_ledger_version(conn, &uber).await?;
        CustomerCreditAccountRow::bump_ledger_version(conn, &uber).await?;
        credit_inputs::record_ledger_change(conn, &uber).await?;
        credit_inputs::record_topup_runtime(conn, &uber).await?;
        credit_inputs::record_topup_runtime(conn, &uber).await?;
        credit_inputs::bump_customers(conn, &[&spotify, &uber, &spotify]).await?;
        credit_inputs::bump_customer(conn, &uber).await?;
        Ok(())
    })
    .await
    .map_err(test_error)?;

    let rows = feed_since(&mut conn, mark).await?;
    assert_eq!(rows.len(), 4, "{rows:?}");
    let uber = Subject::Customer(CUST_UBER_ID);
    assert_recorded(&rows, uber, CreditInputChangeKind::Ledger, "ledger");
    assert_recorded(&rows, uber, CreditInputChangeKind::TopUpRuntime, "top-up");
    assert_recorded(&rows, uber, CreditInputChangeKind::Inputs, "inputs");
    assert_recorded(
        &rows,
        Subject::Customer(CUST_SPOTIFY_ID),
        CreditInputChangeKind::Inputs,
        "inputs",
    );
    Ok(())
}

#[derive(QueryableByName)]
struct Blocker {
    #[diesel(sql_type = sql_types::Integer)]
    pid: i32,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Text>)]
    datname: Option<String>,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Text>)]
    state: Option<String>,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Text>)]
    backend_xmin: Option<String>,
    #[diesel(sql_type = sql_types::Nullable<sql_types::Text>)]
    query: Option<String>,
}

/// Waits, up to a deadline, until the cluster's oldest open transaction is past `xid`: only then
/// may a poll return rows at `xid`. Fails with the backends holding it back.
async fn wait_until_pollable(conn: &mut PgConn, xid: i64) -> TestResult {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    loop {
        let xmin: Scalar =
            sql_query("SELECT pg_snapshot_xmin(pg_current_snapshot())::text::bigint AS value")
                .get_result(conn)
                .await
                .map_err(test_error)?;
        if xmin.value > xid {
            return Ok(());
        }
        if tokio::time::Instant::now() > deadline {
            let blockers: Vec<Blocker> = sql_query(
                "SELECT pid, datname::text, state, backend_xmin::text, left(query, 200) AS query
                 FROM pg_stat_activity
                 WHERE pid <> pg_backend_pid()
                   AND (backend_xid IS NOT NULL OR backend_xmin IS NOT NULL)
                 ORDER BY xact_start",
            )
            .get_results(conn)
            .await
            .map_err(test_error)?;
            let blockers: Vec<String> = blockers
                .iter()
                .map(|b| {
                    format!(
                        "pid {} db {:?} state {:?} xmin {:?}: {:?}",
                        b.pid, b.datname, b.state, b.backend_xmin, b.query
                    )
                })
                .collect();
            return Err(
                format!("xmin stayed at or below {xid}; held back by {blockers:#?}").into(),
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Every committed feed row after `cursor` the poll returns, with the cursor past them.
async fn poll_all(
    conn: &mut PgConn,
    mut cursor: CreditFeedCursor,
) -> TestResult<(Vec<CreditInputChangeRow>, CreditFeedCursor)> {
    let mut rows = Vec::new();
    loop {
        let page = credit_inputs::poll_changes(conn, None, cursor, 500)
            .await
            .map_err(test_error)?;
        let Some(last) = page.last() else {
            return Ok((rows, cursor));
        };
        cursor = last.cursor();
        rows.extend(page);
    }
}

#[tokio::test]
async fn the_poll_never_passes_a_transaction_still_open() -> TestResult {
    let setup = setup().await;
    let mut poller = setup.store.get_conn().await.map_err(test_error)?;
    let mut first = setup.store.get_conn().await.map_err(test_error)?;
    let mut second = setup.store.get_conn().await.map_err(test_error)?;
    let mark = feed_mark(&mut poller).await?;

    // The first transaction takes its id (writes its row) before the second, and stays open.
    first.batch_execute("BEGIN").await.map_err(test_error)?;
    let uber = CustomerRow::lock(&mut first, TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?;
    credit_inputs::bump_customer(&mut first, &uber)
        .await
        .map_err(test_error)?;
    second
        .transaction::<(), DatabaseErrorContainer, _>(async |conn| {
            let spotify = CustomerRow::lock(conn, TENANT_ID, CUST_SPOTIFY_ID).await?;
            credit_inputs::bump_customer(conn, &spotify).await
        })
        .await
        .map_err(test_error)?;

    let (seen, _) = poll_all(&mut poller, CreditFeedCursor::default()).await?;
    assert!(
        seen.iter().all(|row| row.id <= mark),
        "the committed later row waits for the open earlier one: {seen:?}"
    );

    first.batch_execute("COMMIT").await.map_err(test_error)?;
    let written = feed_since(&mut poller, mark).await?;
    let newest = written
        .iter()
        .map(|row| row.store_xid)
        .max()
        .ok_or("rows")?;
    wait_until_pollable(&mut poller, newest).await?;
    let (seen, _) = poll_all(&mut poller, CreditFeedCursor::default()).await?;
    let customers: Vec<_> = seen
        .iter()
        .filter(|row| row.id > mark)
        .map(|row| row.customer_id)
        .collect();
    assert_eq!(customers, vec![Some(CUST_UBER_ID), Some(CUST_SPOTIFY_ID)]);
    Ok(())
}

#[tokio::test]
async fn the_poll_returns_rows_below_an_open_transaction() -> TestResult {
    let setup = setup().await;
    let mut poller = setup.store.get_conn().await.map_err(test_error)?;
    let mut open = setup.store.get_conn().await.map_err(test_error)?;
    let mark = feed_mark(&mut poller).await?;

    setup
        .store
        .transaction(|conn| {
            async move {
                let spotify = CustomerRow::lock(conn, TENANT_ID, CUST_SPOTIFY_ID).await?;
                Ok(credit_inputs::bump_customer(conn, &spotify).await?)
            }
            .scope_boxed()
        })
        .await
        .map_err(test_error)?;
    let committed = feed_since(&mut poller, mark).await?;
    let committed_xid = <[FeedRow]>::first(&committed).ok_or("row")?.store_xid;

    // Opened after the commit, so its id is larger; it stays open during the poll.
    open.batch_execute("BEGIN").await.map_err(test_error)?;
    let uber = CustomerRow::lock(&mut open, TENANT_ID, CUST_UBER_ID)
        .await
        .map_err(test_error)?;
    credit_inputs::bump_customer(&mut open, &uber)
        .await
        .map_err(test_error)?;

    wait_until_pollable(&mut poller, committed_xid).await?;
    let (seen, _) = poll_all(&mut poller, CreditFeedCursor::default()).await?;
    let mine: Vec<_> = seen.iter().filter(|row| row.id > mark).collect();
    assert_eq!(mine.len(), 1, "{mine:?}");
    assert_eq!(mine[0].customer_id, Some(CUST_SPOTIFY_ID));
    open.batch_execute("COMMIT").await.map_err(test_error)?;
    Ok(())
}

#[derive(QueryableByName)]
struct InsertedRow {
    #[diesel(sql_type = sql_types::Text)]
    partition: String,
    #[diesel(sql_type = sql_types::BigInt)]
    store_xid: i64,
    #[diesel(sql_type = sql_types::BigInt)]
    id: i64,
}

async fn insert_row(conn: &mut PgConn, created_on: NaiveDate) -> TestResult<InsertedRow> {
    sql_query(
        "INSERT INTO credit_input_change (shard, tenant_id, customer_id, kind, created_on)
         VALUES (credit_input_shard($1), $1, $2, $4, $3)
         RETURNING tableoid::regclass::text AS partition, store_xid::text::bigint AS store_xid, id",
    )
    .bind::<sql_types::Uuid, _>(TENANT_ID.as_uuid())
    .bind::<sql_types::Uuid, _>(CUST_UBER_ID.as_uuid())
    .bind::<sql_types::Date, _>(created_on)
    .bind::<CreditInputChangeKindEnum, _>(CreditInputChangeKind::Inputs)
    .get_result(conn)
    .await
    .map_err(test_error)
}

async fn first_partition_day(conn: &mut PgConn) -> TestResult<NaiveDate> {
    let days = credit_inputs::partition_days(conn)
        .await
        .map_err(test_error)?;
    assert_eq!(
        days.len(),
        FEED_DAYS_AHEAD as usize + 1,
        "the migration creates them"
    );
    Ok(*<[NaiveDate]>::first(&days).ok_or("partitions")?)
}

#[tokio::test]
async fn a_row_outside_the_partitions_lands_in_default() -> TestResult {
    let setup = setup().await;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let first_day = first_partition_day(&mut conn).await?;
    let row = insert_row(&mut conn, first_day + Days::new(400)).await?;
    assert_eq!(row.partition, "credit_input_change_default");
    assert_eq!(
        credit_inputs::default_partition_rows(&mut conn)
            .await
            .map_err(test_error)?,
        1
    );
    Ok(())
}

#[tokio::test]
async fn maintenance_creates_ahead_and_drops_behind_recording_the_last_row() -> TestResult {
    let setup = setup().await;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let first_day = first_partition_day(&mut conn).await?;
    assert_eq!(
        credit_inputs::retention_floor(&mut conn)
            .await
            .map_err(test_error)?,
        None
    );
    let old = insert_row(&mut conn, first_day).await?;
    let old_cursor = CreditFeedCursor {
        store_xid: old.store_xid,
        id: old.id,
    };

    // Twenty days on: the first days fell out of retention, the next ones are missing.
    let today = first_day + Days::new(20);
    let outcome = setup.store.maintain_credit_input_feed(today).await;

    assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
    let cutoff = today - Days::new(FEED_RETENTION_DAYS);
    let expected: Vec<NaiveDate> = first_day
        .iter_days()
        .take_while(|day| *day < cutoff)
        .collect();
    assert_eq!(outcome.dropped, expected);
    assert_eq!(outcome.days_ahead, FEED_DAYS_AHEAD as i64);
    assert_eq!(
        outcome.created.last().copied(),
        Some(today + Days::new(FEED_DAYS_AHEAD))
    );
    let now = credit_inputs::partition_days(&mut conn)
        .await
        .map_err(test_error)?;
    assert!(now.iter().all(|day| *day >= cutoff));
    assert!(
        credit_inputs::detached_partition_days(&mut conn)
            .await
            .map_err(test_error)?
            .is_empty()
    );
    let floor = credit_inputs::retention_floor(&mut conn)
        .await
        .map_err(test_error)?
        .ok_or("the dropped days are recorded")?;
    assert!(floor >= old_cursor);
    let before_old = CreditFeedCursor {
        store_xid: old.store_xid,
        id: old.id - 1,
    };
    assert!(
        credit_inputs::cursor_missed_rows(&mut conn, before_old)
            .await
            .map_err(test_error)?
    );

    let again = setup.store.maintain_credit_input_feed(today).await;
    assert!(again.created.is_empty() && again.dropped.is_empty() && again.failures.is_empty());
    Ok(())
}

#[tokio::test]
async fn maintenance_leaves_a_day_with_rows_in_default_and_purges_old_ones() -> TestResult {
    let setup = setup().await;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let first_day = first_partition_day(&mut conn).await?;
    let today = first_day + Days::new(10);
    let held = first_day + Days::new(20);
    let in_default = insert_row(&mut conn, held).await?;
    assert_eq!(in_default.partition, "credit_input_change_default");
    let ancient_day = first_day - Days::new(30);
    let ancient = insert_row(&mut conn, ancient_day).await?;
    assert_eq!(ancient.partition, "credit_input_change_default");

    let outcome = setup.store.maintain_credit_input_feed(today).await;

    assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
    assert_eq!(outcome.skipped, vec![held]);
    assert!(!outcome.created.contains(&held));
    assert_eq!(
        outcome.default_rows,
        Some(1),
        "the old row is purged, the held one stays"
    );
    #[derive(QueryableByName)]
    struct Retained {
        #[diesel(sql_type = sql_types::BigInt)]
        store_xid: i64,
        #[diesel(sql_type = sql_types::BigInt)]
        id: i64,
    }
    let retained: Retained = sql_query(
        "SELECT max_store_xid::text::bigint AS store_xid, max_id AS id
         FROM credit_input_change_retention WHERE partition_day = $1",
    )
    .bind::<sql_types::Date, _>(ancient_day)
    .get_result(&mut conn)
    .await
    .map_err(test_error)?;
    assert_eq!(
        (retained.store_xid, retained.id),
        (ancient.store_xid, ancient.id)
    );
    #[derive(QueryableByName)]
    struct Guards {
        #[diesel(sql_type = sql_types::BigInt)]
        count: i64,
    }
    let guards: Guards = sql_query(
        "SELECT count(*) AS count FROM pg_constraint
         WHERE conrelid = 'credit_input_change_default'::regclass AND contype = 'c'
           AND conname LIKE 'credit_input_change_default_not_%'",
    )
    .get_result(&mut conn)
    .await
    .map_err(test_error)?;
    assert_eq!(guards.count, 0, "no guard outlives its attach");
    Ok(())
}

#[tokio::test]
async fn attaching_a_future_day_lets_writers_run() -> TestResult {
    let setup = setup().await;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let mut writer = setup.store.get_conn().await.map_err(test_error)?;
    let first_day = first_partition_day(&mut conn).await?;
    let done = AtomicBool::new(false);

    let maintain = async {
        let outcome = setup
            .store
            .maintain_credit_input_feed(first_day + Days::new(5))
            .await;
        done.store(true, Ordering::SeqCst);
        outcome
    };
    let write = async {
        let mut written = 0;
        while !AtomicBool::load(&done, Ordering::SeqCst) || written == 0 {
            let row = insert_row(&mut writer, first_day).await?;
            assert_ne!(row.partition, "credit_input_change_default");
            written += 1;
        }
        Ok::<_, Box<dyn std::error::Error>>(written)
    };
    let (outcome, written) = tokio::time::timeout(Duration::from_secs(60), async {
        tokio::join!(maintain, write)
    })
    .await
    .map_err(|_| "maintenance or the writer stalled")?;
    assert!(written? > 0);
    assert!(outcome.failures.is_empty(), "{:?}", outcome.failures);
    assert_eq!(outcome.created.len(), 5);
    Ok(())
}

#[tokio::test]
async fn overlapping_customer_and_tenant_bumps_do_not_deadlock() -> TestResult {
    let setup = setup().await;
    let mut conn = setup.store.get_conn().await.map_err(test_error)?;
    let mark = feed_mark(&mut conn).await?;
    // Both hold their first customer before either goes on; the first then needs the second's
    // customer, which is released only by the second's commit, after its tenant bump.
    let held = Barrier::new(2);
    let first = setup.store.transaction_bumping_tenant(TENANT_ID, |conn| {
        let held = &held;
        async move {
            let uber = CustomerRow::lock(conn, TENANT_ID, CUST_UBER_ID).await?;
            credit_inputs::bump_customer(conn, &uber).await?;
            held.wait().await;
            let spotify = CustomerRow::lock(conn, TENANT_ID, CUST_SPOTIFY_ID).await?;
            credit_inputs::bump_customer(conn, &spotify).await?;
            Ok(())
        }
        .scope_boxed()
    });
    let second = setup.store.transaction_bumping_tenant(TENANT_ID, |conn| {
        let held = &held;
        async move {
            let spotify = CustomerRow::lock(conn, TENANT_ID, CUST_SPOTIFY_ID).await?;
            credit_inputs::bump_customer(conn, &spotify).await?;
            held.wait().await;
            Ok(())
        }
        .scope_boxed()
    });
    let (first, second) = tokio::time::timeout(Duration::from_secs(60), async {
        tokio::join!(first, second)
    })
    .await
    .map_err(|_| "the transactions stalled")?;
    first.map_err(test_error)?;
    second.map_err(test_error)?;

    let rows = feed_since(&mut conn, mark).await?;
    assert_eq!(
        rows.iter()
            .filter(|row| row.kind == CreditInputChangeKind::Tenant)
            .count(),
        2
    );
    Ok(())
}

/// The same interleaving with the tenant bumped before the second customer is taken: Postgres
/// detects the cycle. The harness above would fail the same way if the bump were not last.
#[tokio::test]
async fn a_tenant_bump_before_another_lock_deadlocks() -> TestResult {
    let setup = setup().await;
    let held = Barrier::new(2);
    let tenant_taken = Barrier::new(2);
    let early = setup.store.transaction(|conn| {
        let (held, tenant_taken) = (&held, &tenant_taken);
        async move {
            let uber = CustomerRow::lock(conn, TENANT_ID, CUST_UBER_ID).await?;
            held.wait().await;
            credit_inputs::bump_tenant(conn, TENANT_ID).await?;
            tenant_taken.wait().await;
            let spotify = CustomerRow::lock(conn, TENANT_ID, CUST_SPOTIFY_ID).await?;
            credit_inputs::bump_customer(conn, &uber).await?;
            credit_inputs::bump_customer(conn, &spotify).await?;
            Ok(())
        }
        .scope_boxed()
    });
    let late = setup.store.transaction(|conn| {
        let (held, tenant_taken) = (&held, &tenant_taken);
        async move {
            CustomerRow::lock(conn, TENANT_ID, CUST_SPOTIFY_ID).await?;
            held.wait().await;
            tenant_taken.wait().await;
            credit_inputs::bump_tenant(conn, TENANT_ID).await?;
            Ok(())
        }
        .scope_boxed()
    });
    let (early, late) =
        tokio::time::timeout(Duration::from_secs(60), async { tokio::join!(early, late) })
            .await
            .map_err(|_| "Postgres did not break the cycle")?;
    let failed = [format!("{early:?}"), format!("{late:?}")];
    assert!(
        failed.iter().any(|outcome| outcome.contains("deadlock")),
        "one side is the deadlock victim: {failed:?}"
    );
    Ok(())
}
