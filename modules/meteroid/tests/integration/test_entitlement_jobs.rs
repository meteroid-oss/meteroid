//! The entitlement jobs against the store (ADR 0011 §3.10, §7): the validity scan's marks and
//! cursor, and the dry run's load and write. Their pure parts have unit tests in the workers.
use chrono::{DateTime, Duration, Utc};
use common_domain::ids::{BaseId, CustomerId, FeatureId, PlanVersionId, SubscriptionId};
use diesel::QueryableByName;
use diesel::sql_types::{Nullable, Text, Timestamptz, Uuid as SqlUuid};
use diesel_async::RunQueryDsl;
use meteroid_store::store::PgConn;
use meteroid_workers::misc::entitlement_validity_scan::scan;
use uuid::Uuid;

use crate::data::ids;
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;

#[derive(QueryableByName)]
struct Sub {
    #[diesel(sql_type = SqlUuid)]
    id: SubscriptionId,
    #[diesel(sql_type = SqlUuid)]
    customer_id: CustomerId,
    #[diesel(sql_type = SqlUuid)]
    plan_version_id: PlanVersionId,
}

#[derive(QueryableByName)]
struct Id {
    #[diesel(sql_type = SqlUuid)]
    id: Uuid,
}

#[derive(QueryableByName)]
struct Cursor {
    #[diesel(sql_type = Timestamptz)]
    checked_until: DateTime<Utc>,
}

async fn active_subscription(conn: &mut PgConn) -> Sub {
    diesel::sql_query(
        "SELECT id, customer_id, plan_version_id FROM subscription
         WHERE status = 'ACTIVE' AND tenant_id = $1 ORDER BY id LIMIT 1",
    )
    .bind::<SqlUuid, _>(ids::TENANT_ID.as_uuid())
    .get_result(conn)
    .await
    .expect("a seeded active subscription")
}

async fn feature(conn: &mut PgConn, code: &str, feature_type: &str) -> FeatureId {
    let id: Id = diesel::sql_query(format!(
        "INSERT INTO feature (tenant_id, name, code, feature_type, kind)
         VALUES ($1, $2, $2, '{feature_type}',
                 CASE '{feature_type}' WHEN 'METERED' THEN 'LIMIT' ELSE 'BOOLEAN' END::\"FeatureKindEnum\")
         RETURNING id"
    ))
    .bind::<SqlUuid, _>(ids::TENANT_ID.as_uuid())
    .bind::<Text, _>(code)
    .get_result(conn)
    .await
    .unwrap();
    FeatureId::from(id.id)
}

/// A row of its own lineage, written as the store writes one, its anchor included.
#[allow(clippy::too_many_arguments)]
async fn row(
    conn: &mut PgConn,
    feature_id: FeatureId,
    entity_type: &str,
    entity_id: Uuid,
    value: serde_json::Value,
    valid_from: Option<DateTime<Utc>>,
    valid_until: Option<DateTime<Utc>>,
    revoked_at: Option<DateTime<Utc>>,
) {
    diesel::sql_query(format!(
        "INSERT INTO entitlement (id, tenant_id, feature_id, entity_id, entity_type, mode, value,
                                  valid_from, valid_until, revoked_at, lineage_id,
                                  lineage_created_at, lineage_anchor, effect)
         SELECT n.id, $1, $2, $3, '{entity_type}', 'OVERRIDE', $4, $5, $6, $7, n.id, now(),
                LEAST(COALESCE($5, now()), now()), 'SET'
         FROM (SELECT gen_random_uuid() AS id) n"
    ))
    .bind::<SqlUuid, _>(ids::TENANT_ID.as_uuid())
    .bind::<SqlUuid, _>(feature_id.as_uuid())
    .bind::<SqlUuid, _>(entity_id)
    .bind::<diesel::sql_types::Jsonb, _>(value)
    .bind::<Nullable<Timestamptz>, _>(valid_from)
    .bind::<Nullable<Timestamptz>, _>(valid_until)
    .bind::<Nullable<Timestamptz>, _>(revoked_at)
    .execute(conn)
    .await
    .unwrap();
}

async fn marked(conn: &mut PgConn) -> Vec<CustomerId> {
    let rows: Vec<Id> = diesel::sql_query("SELECT customer_id AS id FROM credit_recompute_mark")
        .load(conn)
        .await
        .unwrap();
    rows.into_iter().map(|r| CustomerId::from(r.id)).collect()
}

async fn clear_marks(conn: &mut PgConn) {
    diesel::sql_query("DELETE FROM credit_recompute_mark")
        .execute(conn)
        .await
        .unwrap();
}

async fn cursor(conn: &mut PgConn) -> DateTime<Utc> {
    diesel::sql_query("SELECT checked_until FROM entitlement_schedule_cursor")
        .get_result::<Cursor>(conn)
        .await
        .unwrap()
        .checked_until
}

#[tokio::test]
async fn the_validity_scan_marks_the_customers_a_bound_reaches_and_never_skips_one() {
    helpers::init::logging();
    let db = meteroid_it::container::create_test_database().await;
    let setup = meteroid_it::container::start_meteroid(db, SeedLevel::SUBSCRIPTIONS).await;
    let store = &setup.store;
    let mut conn = store.get_conn().await.unwrap();

    let t0 = DateTime::from_timestamp(Utc::now().timestamp() - 3_600, 0).unwrap();
    let minutes = |m: i64| t0 + Duration::minutes(m);
    diesel::sql_query("UPDATE entitlement_schedule_cursor SET checked_until = $1")
        .bind::<Timestamptz, _>(t0)
        .execute(&mut conn)
        .await
        .unwrap();

    let sub = active_subscription(&mut conn).await;
    let others: Vec<CustomerId> = [ids::CUST_UBER_ID, ids::CUST_COMODO_ID, ids::CUST_SPOTIFY_ID]
        .into_iter()
        .filter(|c| *c != sub.customer_id)
        .collect();
    let (reached, untouched) = (others[0], others[1]);
    let on = serde_json::json!({"type": "boolean", "enabled": true});
    let sso = feature(&mut conn, "sso_dated", "BOOLEAN").await;
    let customer = |c: CustomerId| c.as_uuid();

    row(
        &mut conn,
        sso,
        "CUSTOMER",
        customer(reached),
        on.clone(),
        Some(minutes(10)),
        None,
        None,
    )
    .await;
    row(
        &mut conn,
        sso,
        "SUBSCRIPTION",
        sub.id.as_uuid(),
        on.clone(),
        None,
        Some(minutes(20)),
        None,
    )
    .await;
    let before_cursor = Some(t0 - Duration::days(1));
    row(
        &mut conn,
        sso,
        "CUSTOMER",
        customer(untouched),
        on.clone(),
        before_cursor,
        None,
        None,
    )
    .await;
    let revoked = Some(minutes(5));
    let other = feature(&mut conn, "sso_revoked", "BOOLEAN").await;
    row(
        &mut conn,
        other,
        "CUSTOMER",
        customer(untouched),
        on.clone(),
        Some(minutes(10)),
        None,
        revoked,
    )
    .await;
    clear_marks(&mut conn).await;

    assert!(scan(store, minutes(30)).await.unwrap() >= 2);
    let marks = marked(&mut conn).await;
    assert!(marks.contains(&reached), "{marks:?}");
    assert!(marks.contains(&sub.customer_id), "{marks:?}");
    assert!(!marks.contains(&untouched), "{marks:?}");
    assert_eq!(cursor(&mut conn).await, minutes(30));

    // A scan behind the cursor (a clock step back, a restart) marks nothing and keeps it.
    clear_marks(&mut conn).await;
    assert_eq!(scan(store, minutes(15)).await.unwrap(), 0);
    assert_eq!(cursor(&mut conn).await, minutes(30));

    // `(cursor, now]`: a bound on the cursor was handled already.
    let plan_feature = feature(&mut conn, "sso_plan", "BOOLEAN").await;
    let on_cursor = Some(minutes(30));
    row(
        &mut conn,
        plan_feature,
        "CUSTOMER",
        customer(untouched),
        on.clone(),
        on_cursor,
        None,
        None,
    )
    .await;
    assert_eq!(scan(store, minutes(35)).await.unwrap(), 0);
    assert!(marked(&mut conn).await.is_empty());

    // One on `now` is handled now, and a plan version's reaches its subscribers.
    let version = sub.plan_version_id.as_uuid();
    row(
        &mut conn,
        plan_feature,
        "PLAN_VERSION",
        version,
        on.clone(),
        Some(minutes(40)),
        None,
        None,
    )
    .await;
    assert!(scan(store, minutes(40)).await.unwrap() >= 1);
    assert!(marked(&mut conn).await.contains(&sub.customer_id));
    assert_eq!(cursor(&mut conn).await, minutes(40));

    // An add-on ending later is still in effect: its rows reach the subscriber.
    clear_marks(&mut conn).await;
    let add_on: Id = diesel::sql_query(
        "INSERT INTO add_on (id, name, tenant_id, product_id, price_id, fee_structure)
         SELECT gen_random_uuid(), 'Pack', tenant_id, product_id, id,
                COALESCE((SELECT pr.fee_structure FROM product pr WHERE pr.id = price.product_id), (SELECT pc.fee_structure FROM price_component pc JOIN plan_component_price pcp ON pcp.plan_component_id = pc.id WHERE pcp.price_id = price.id LIMIT 1), '{\"type\": \"Rate\"}'::jsonb)
         FROM price WHERE tenant_id = $1 LIMIT 1 RETURNING id",
    )
    .bind::<SqlUuid, _>(ids::TENANT_ID.as_uuid())
    .get_result(&mut conn)
    .await
    .unwrap();
    diesel::sql_query(
        "INSERT INTO subscription_add_on (id, name, subscription_id, add_on_id, period,
             effective_from, effective_to, created_at)
         VALUES (gen_random_uuid(), 'Pack', $1, $2, 'MONTHLY', current_date - 1, current_date + 30,
                 now())",
    )
    .bind::<SqlUuid, _>(sub.id.as_uuid())
    .bind::<SqlUuid, _>(add_on.id)
    .execute(&mut conn)
    .await
    .unwrap();
    let pack_feature = feature(&mut conn, "sso_pack", "BOOLEAN").await;
    row(
        &mut conn,
        pack_feature,
        "ADD_ON",
        add_on.id,
        on.clone(),
        Some(minutes(45)),
        None,
        None,
    )
    .await;
    assert!(scan(store, minutes(50)).await.unwrap() >= 1);
    assert!(marked(&mut conn).await.contains(&sub.customer_id));
}

/// Phase 2's rekey copies a subscription monitor's alert state to the customer level, kept for
/// the previous release during the rollout, and only hashes customers with monitor state.
#[tokio::test]
async fn the_phase_two_rekey_copies_alert_state_to_the_customer_level() {
    use diesel_async::SimpleAsyncConnection;
    use meteroid_store::domain::entitlement_fold::EvalLevel;
    use meteroid_store::usage_controls_support::limits::level_monitor_id;
    helpers::init::logging();
    let db = meteroid_it::container::create_test_database().await;
    let setup = meteroid_it::container::start_meteroid(db, SeedLevel::SUBSCRIPTIONS).await;
    let mut conn = setup.store.get_conn().await.unwrap();
    clear_marks(&mut conn).await;

    let sub = active_subscription(&mut conn).await;
    let api_calls = feature(&mut conn, "api_calls_rekey", "METERED").await;
    let metered = serde_json::json!({"type": "metered", "limit": "10", "enabled": true});
    row(
        &mut conn,
        api_calls,
        "SUBSCRIPTION",
        sub.id.as_uuid(),
        metered,
        None,
        None,
        None,
    )
    .await;
    let old_id = Uuid::new_v5(&api_calls.as_uuid(), sub.id.as_uuid().as_bytes());
    let unrelated = Uuid::now_v7();
    conn.batch_execute(&format!(
        "INSERT INTO usage_monitor_state (monitor_id, window_start, tenant_id, customer_id, \
         monitor_kind, window_end, used, usage_as_of, computed_at) VALUES \
         ('{old_id}', now(), '{tenant}', '{customer}', 'entitlement', now() + interval '1 day', 3, \
          now(), now());
         INSERT INTO usage_alert_state (monitor_id, window_key, level, tenant_id) VALUES \
         ('{old_id}', 'w', 80, '{tenant}'), ('{unrelated}', 'w', 80, '{tenant}');",
        tenant = ids::TENANT_ID.as_uuid(),
        customer = sub.customer_id.as_uuid(),
    ))
    .await
    .unwrap();

    let up = include_str!("../../migrations/diesel/2026-11-26-011437_entitlements_v2/up.sql");
    let rekey = &up[up.find("-- UUID v5 over raw bytes").unwrap()
        ..up.find("CREATE TYPE \"EntitlementEffectEnum\"").unwrap()];
    conn.batch_execute(rekey).await.unwrap();

    let alerts: Vec<Id> =
        diesel::sql_query("SELECT monitor_id AS id FROM usage_alert_state ORDER BY 1")
            .load(&mut conn)
            .await
            .unwrap();
    let mut expected = vec![
        level_monitor_id(api_calls, sub.customer_id, EvalLevel::Customer),
        old_id,
        unrelated,
    ];
    expected.sort();
    assert_eq!(
        alerts.into_iter().map(|a| a.id).collect::<Vec<_>>(),
        expected
    );
    let states: Vec<Id> = diesel::sql_query("SELECT monitor_id AS id FROM usage_monitor_state")
        .load(&mut conn)
        .await
        .unwrap();
    assert!(states.iter().all(|s| s.id != old_id));
    assert_eq!(marked(&mut conn).await, vec![sub.customer_id]);
}

/// The migration's UUID v5 matches `Uuid::new_v5` on a pinned vector.
#[tokio::test]
async fn the_rekey_uuid_v5_matches_rust() {
    use diesel_async::SimpleAsyncConnection;
    helpers::init::logging();
    let db = meteroid_it::container::create_test_database().await;
    let setup = meteroid_it::container::start_meteroid(db, SeedLevel::MINIMAL).await;
    let mut conn = setup.store.get_conn().await.unwrap();
    let up = include_str!("../../migrations/diesel/2026-11-26-011437_entitlements_v2/up.sql");
    conn.batch_execute(
        &up[up.find("-- UUID v5 over raw bytes").unwrap()..up.find("-- EE monitors move").unwrap()],
    )
    .await
    .unwrap();
    let (ns, name) = (
        Uuid::parse_str("00000000-0000-0000-0000-00000000f001").unwrap(),
        Uuid::parse_str("00000000-0000-0000-0000-0000000005a1").unwrap(),
    );
    let id: Id = diesel::sql_query("SELECT pg_temp.uuid_v5($1, uuid_send($2)) AS id")
        .bind::<SqlUuid, _>(ns)
        .bind::<SqlUuid, _>(name)
        .get_result(&mut conn)
        .await
        .unwrap();
    let pinned = Uuid::parse_str("db80be34-523c-57a2-8779-3e4095381496").unwrap();
    assert_eq!(id.id, pinned);
    assert_eq!(Uuid::new_v5(&ns, name.as_bytes()), pinned);
}

/// No trigger of v2 is left on the tables it writes: every write path sets what triggers used to.
/// The activation triggers are main's.
#[tokio::test]
async fn no_trigger_remains_on_entitlement_tables() {
    #[derive(QueryableByName)]
    struct Trigger {
        #[diesel(sql_type = Text)]
        name: String,
    }
    helpers::init::logging();
    let db = meteroid_it::container::create_test_database().await;
    let setup = meteroid_it::container::start_meteroid(db, SeedLevel::MINIMAL).await;
    let mut conn = setup.store.get_conn().await.unwrap();
    let triggers: Vec<Trigger> = diesel::sql_query(
        "SELECT tgrelid::regclass::text || '.' || tgname AS name FROM pg_trigger \
         WHERE NOT tgisinternal \
           AND tgrelid IN ('subscription'::regclass, 'entitlement'::regclass, \
                           'feature'::regclass)",
    )
    .load(&mut conn)
    .await
    .unwrap();
    let mut names = triggers.into_iter().map(|t| t.name).collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [
            "subscription.trg_subscription_activity_forget",
            "subscription.trg_subscription_emit_activated_event_ins",
            "subscription.trg_subscription_emit_activated_event_upd",
            "subscription.trg_subscription_set_activated_at",
            "subscription.trg_subscription_touch",
        ]
    );
}

/// Every insert path anchors its lineage, as the trigger did: a new lineage at the earlier of its
/// `valid_from` and its creation, a successor at its lineage's first anchor, a batch row by row,
/// and a plan version copy as a new lineage.
#[tokio::test]
async fn every_insert_path_anchors_its_lineage() {
    use common_domain::ids::EntitlementId;
    use diesel_models::entitlements::{EntitlementRow, EntitlementRowNew};
    use diesel_models::enums::{
        EntitlementEffectEnum, EntitlementEntityTypeEnum, EntitlementLevelEnum, EntitlementModeEnum,
    };
    helpers::init::logging();
    let db = meteroid_it::container::create_test_database().await;
    let setup = meteroid_it::container::start_meteroid(db, SeedLevel::PLANS).await;
    let mut conn = setup.store.get_conn().await.unwrap();
    let flag = feature(&mut conn, "anchored_flag", "BOOLEAN").await;
    let now = chrono::SubsecRound::trunc_subsecs(Utc::now(), 6);
    let new_row = |entity_type, entity_id, valid_from: Option<DateTime<Utc>>| {
        let id = EntitlementId::new();
        EntitlementRowNew {
            id,
            tenant_id: ids::TENANT_ID,
            feature_id: flag,
            entity_id,
            entity_type,
            mode: EntitlementModeEnum::Override,
            value: serde_json::json!({"type": "boolean", "enabled": true}),
            lineage_id: id,
            lineage_created_at: now,
            created_at: now,
            effect: EntitlementEffectEnum::Set,
            level: EntitlementLevelEnum::Feature,
            level_type_id: None,
            resource_id: None,
            condition: None,
            condition_hash: String::new(),
            valid_from,
            valid_until: None,
            reason: None,
            created_by: None,
            created_by_kind: None,
            policy_override: None,
        }
    };
    let customer = ids::CUST_UBER_ID.as_uuid();
    let earlier = now - Duration::days(10);

    let dated = new_row(EntitlementEntityTypeEnum::Customer, customer, Some(earlier))
        .insert(&mut conn)
        .await
        .unwrap();
    assert_eq!(dated.lineage_anchor, earlier, "a new lineage dated earlier");
    let mut successor = new_row(
        EntitlementEntityTypeEnum::Customer,
        customer,
        Some(now + Duration::days(1)),
    );
    successor.lineage_id = dated.lineage_id;
    let successor = successor.insert_successor(&mut conn, None).await.unwrap();
    assert_eq!(
        successor.lineage_anchor, earlier,
        "an edit keeps its lineage's anchor"
    );

    let version = ids::PLAN_VERSION_1_LEETCODE_ID;
    let batch = EntitlementRowNew::insert_batch(
        &[
            new_row(
                EntitlementEntityTypeEnum::PlanVersion,
                version.as_uuid(),
                Some(now + Duration::days(5)),
            ),
            new_row(EntitlementEntityTypeEnum::Plan, Uuid::now_v7(), None),
        ],
        &mut conn,
    )
    .await
    .unwrap();
    assert!(
        batch.iter().all(|row| row.lineage_anchor == now),
        "undated or dated later: its creation"
    );

    let copy = ids::PLAN_VERSION_2_LEETCODE_ID;
    EntitlementRow::clone_all_for_plan_version(&mut conn, version, copy, ids::TENANT_ID)
        .await
        .unwrap();
    let copied: Cursor = diesel::sql_query(
        "SELECT lineage_anchor AS checked_until FROM entitlement WHERE entity_id = $1",
    )
    .bind::<SqlUuid, _>(copy.as_uuid())
    .get_result(&mut conn)
    .await
    .unwrap();
    assert_eq!(copied.checked_until, now, "a copy anchors a new lineage");
}
