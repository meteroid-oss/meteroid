//! Advanced list filters (custom properties + native customer columns) against real Postgres.
//!
//! Custom-property definitions are cached per `(tenant, entity type)` in-process, and every test
//! database reuses the seeded `TENANT_ID`. To keep parallel tests from reading each other's
//! cached definitions, the customer test runs in a fresh tenant, and subscriptions/invoices each
//! use their own entity type within a single test function.

use std::collections::BTreeSet;
use std::str::FromStr;

use common_domain::actor::Actor;
use common_domain::ids::{BaseId, TenantId};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use rstest::rstest;
use serde_json::json;

use meteroid_store::StoreResult;
use meteroid_store::domain::PaginationRequest;
use meteroid_store::domain::custom_property_definitions::{
    CustomPropertyDefinition, CustomPropertyDefinitionNew, PropertyConfig, SelectOption,
};
use meteroid_store::domain::customers::{CustomerNew, CustomerTaxStatus};
use meteroid_store::domain::enums::{
    CustomPropertyEntityType, CustomPropertyType, CustomerType, InvoiceStatusEnum,
    TenantEnvironmentEnum,
};
use meteroid_store::domain::filters::{FieldRef, Filter, FilterOperator as Op};
use meteroid_store::domain::tenants::TenantNew;
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::credit_notes::{CreateCreditNoteParams, CreditType};
use meteroid_store::repositories::{
    CreditNoteInterface, CustomersInterface, InvoiceInterface, SubscriptionInterface,
    TenantInterface,
};
use meteroid_testkit::custom_properties::{
    archive_custom_property_definition, create_custom_property_definition,
};

use crate::data::ids::*;
use crate::harness::{TestEnv, subscription, test_env};

fn f(field: &str, op: Op, values: &[&str]) -> Filter {
    Filter {
        field: FieldRef::parse(field),
        op,
        values: values.iter().map(|s| s.to_string()).collect(),
    }
}

fn page() -> PaginationRequest {
    PaginationRequest {
        page: 0,
        per_page: Some(100),
    }
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[track_caller]
fn assert_invalid<T>(res: StoreResult<T>) {
    match res {
        Ok(_) => panic!("expected InvalidArgument, got Ok"),
        Err(err) => assert!(
            matches!(err.current_context(), StoreError::InvalidArgument(_)),
            "expected InvalidArgument, got {err:?}"
        ),
    }
}

async fn define(
    env: &TestEnv,
    tenant_id: TenantId,
    entity_type: CustomPropertyEntityType,
    key: &str,
    property_type: CustomPropertyType,
    options: &[&str],
) -> CustomPropertyDefinition {
    create_custom_property_definition(
        env.store(),
        CustomPropertyDefinitionNew {
            tenant_id,
            entity_type,
            key: key.to_string(),
            name: key.to_string(),
            description: None,
            property_type,
            config: PropertyConfig {
                options: (!options.is_empty()).then(|| {
                    options
                        .iter()
                        .map(|o| SelectOption {
                            value: o.to_string(),
                            label: None,
                        })
                        .collect()
                }),
                ..Default::default()
            },
            required: false,
            default_value: None,
            display_order: 0,
        },
    )
    .await
}

fn customer(name: &str, currency: &str, props: serde_json::Value) -> CustomerNew {
    CustomerNew {
        name: name.to_string(),
        alias: Some(name.to_lowercase()),
        billing_email: None,
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
        custom_properties: props,
        preferred_locales: vec![],
        customer_type: CustomerType::Company,
        first_name: None,
        last_name: None,
    }
}

#[rstest]
#[tokio::test]
async fn test_customer_list_filters(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    type T = CustomPropertyType;
    const C: CustomPropertyEntityType = CustomPropertyEntityType::Customer;

    // Fresh tenant (see module doc), allowing EUR + USD customers.
    let tenant = store
        .insert_tenant(
            TenantNew {
                name: "Filters".to_string(),
                environment: TenantEnvironmentEnum::Development,
                email_delivery_mode: Some(meteroid_store::domain::enums::EmailDeliveryMode::Off),
                invoicing_entity: None,
            },
            ORGANIZATION_ID,
        )
        .await
        .expect("insert tenant")
        .id;
    {
        use diesel_models::schema::tenant::dsl as t;
        let mut conn = env.conn().await;
        diesel::update(t::tenant.filter(t::id.eq(tenant)))
            .set(t::available_currencies.eq(vec![Some("EUR".to_string()), Some("USD".to_string())]))
            .execute(&mut conn)
            .await
            .unwrap();
    }

    define(&env, tenant, C, "tier", T::Text, &[]).await;
    define(&env, tenant, C, "seats", T::Number, &[]).await;
    define(&env, tenant, C, "signed", T::Date, &[]).await;
    define(&env, tenant, C, "last_seen", T::Datetime, &[]).await;
    define(&env, tenant, C, "vip", T::Boolean, &[]).await;
    define(&env, tenant, C, "region", T::SingleSelect, &["EU", "US"]).await;
    define(&env, tenant, C, "tags", T::MultiSelect, &["a", "b"]).await;
    let legacy = define(&env, tenant, C, "legacy", T::Text, &[]).await;

    let actor = Actor::User { id: USER_ID };
    for c in [
        customer(
            "Acme",
            "EUR",
            json!({
                "tier": "Gold Partner", "seats": 10, "signed": "2026-01-15",
                "last_seen": "2026-03-01T12:00:00Z", "vip": true, "region": "EU",
                "tags": ["a", "b"], "legacy": "x"
            }),
        ),
        customer(
            "Beta",
            "USD",
            json!({
                "tier": "silver", "seats": 3, "signed": "2025-06-01",
                "last_seen": "2025-01-01T08:00:00+02:00", "vip": false, "region": "US",
                "tags": ["b"]
            }),
        ),
        customer("Gamma", "EUR", json!({})),
    ] {
        store
            .insert_customer(actor.clone(), c, tenant)
            .await
            .expect("insert customer");
    }

    let list = |filters: Vec<Filter>| async move {
        store
            .list_customers_with_metrics(tenant, page(), None, None, None, vec![], vec![], filters)
            .await
            .map(|p| {
                p.items
                    .into_iter()
                    .map(|c| c.customer.name)
                    .collect::<BTreeSet<_>>()
            })
    };
    macro_rules! check {
        ($filters:expr, $expected:expr) => {
            assert_eq!(
                list($filters).await.expect("list customers"),
                set(&$expected),
                "filters: {:?}",
                $filters
            );
        };
    }

    check!(Vec::<Filter>::new(), ["Acme", "Beta", "Gamma"]);

    // TEXT
    check!(vec![f("cp:tier", Op::Equals, &["silver"])], ["Beta"]);
    check!(vec![f("cp:tier", Op::Equals, &["Silver"])], [] as [&str; 0]);
    check!(
        vec![f("cp:tier", Op::NotEquals, &["silver"])],
        ["Acme", "Gamma"]
    );
    check!(
        vec![f("cp:tier", Op::In, &["Gold Partner", "silver", "bronze"])],
        ["Acme", "Beta"]
    );
    check!(vec![f("cp:tier", Op::Contains, &["gold"])], ["Acme"]);
    check!(vec![f("cp:tier", Op::Contains, &["L"])], ["Acme", "Beta"]);
    check!(vec![f("cp:tier", Op::Contains, &["%"])], [] as [&str; 0]);
    check!(vec![f("cp:tier", Op::IsSet, &[])], ["Acme", "Beta"]);
    check!(vec![f("cp:tier", Op::IsEmpty, &[])], ["Gamma"]);

    // NUMBER
    check!(vec![f("cp:seats", Op::Equals, &["10"])], ["Acme"]);
    check!(vec![f("cp:seats", Op::Equals, &["10.0"])], ["Acme"]);
    check!(vec![f("cp:seats", Op::NotEquals, &["10"])], ["Beta"]);
    check!(vec![f("cp:seats", Op::Gt, &["5"])], ["Acme"]);
    check!(vec![f("cp:seats", Op::Gte, &["3"])], ["Acme", "Beta"]);
    check!(vec![f("cp:seats", Op::Lt, &["10"])], ["Beta"]);
    check!(vec![f("cp:seats", Op::Lte, &["10"])], ["Acme", "Beta"]);
    check!(
        vec![f("cp:seats", Op::Between, &["3", "10"])],
        ["Acme", "Beta"]
    );
    check!(
        vec![f("cp:seats", Op::Between, &["4", "9"])],
        [] as [&str; 0]
    );
    check!(vec![f("cp:seats", Op::In, &["3", "50"])], ["Beta"]);
    check!(vec![f("cp:seats", Op::IsSet, &[])], ["Acme", "Beta"]);
    check!(vec![f("cp:seats", Op::IsEmpty, &[])], ["Gamma"]);

    // DATE
    check!(vec![f("cp:signed", Op::Equals, &["2026-01-15"])], ["Acme"]);
    check!(vec![f("cp:signed", Op::Before, &["2026-01-01"])], ["Beta"]);
    check!(vec![f("cp:signed", Op::After, &["2025-12-31"])], ["Acme"]);
    check!(
        vec![f("cp:signed", Op::Between, &["2025-01-01", "2025-12-31"])],
        ["Beta"]
    );
    check!(
        vec![f("cp:signed", Op::Between, &["2025-06-01", "2026-01-15"])],
        ["Acme", "Beta"]
    );
    check!(vec![f("cp:signed", Op::IsEmpty, &[])], ["Gamma"]);

    // DATETIME (stored normalized to UTC; filter values normalized the same way)
    check!(
        vec![f("cp:last_seen", Op::After, &["2025-06-01T00:00"])],
        ["Acme"]
    );
    check!(
        vec![f("cp:last_seen", Op::Before, &["2025-01-01T07:00:00Z"])],
        ["Beta"]
    );
    check!(
        vec![f("cp:last_seen", Op::Equals, &["2025-01-01T06:00:00Z"])],
        ["Beta"]
    );

    // BOOLEAN
    check!(vec![f("cp:vip", Op::IsTrue, &[])], ["Acme"]);
    check!(vec![f("cp:vip", Op::IsFalse, &[])], ["Beta"]);
    check!(vec![f("cp:vip", Op::IsSet, &[])], ["Acme", "Beta"]);

    // SELECTS
    check!(vec![f("cp:region", Op::Equals, &["EU"])], ["Acme"]);
    check!(
        vec![f("cp:region", Op::NotEquals, &["EU"])],
        ["Beta", "Gamma"]
    );
    check!(
        vec![f("cp:region", Op::In, &["EU", "US"])],
        ["Acme", "Beta"]
    );
    // Filtering is lenient on option membership (legacy values stay filterable).
    check!(vec![f("cp:region", Op::Equals, &["APAC"])], [] as [&str; 0]);
    check!(vec![f("cp:tags", Op::Contains, &["b"])], ["Acme", "Beta"]);
    check!(vec![f("cp:tags", Op::Contains, &["a"])], ["Acme"]);
    check!(vec![f("cp:tags", Op::In, &["a", "zzz"])], ["Acme"]);
    check!(vec![f("cp:tags", Op::IsEmpty, &[])], ["Gamma"]);

    // Filters AND together.
    check!(
        vec![
            f("cp:seats", Op::Gt, &["1"]),
            f("cp:region", Op::Equals, &["US"])
        ],
        ["Beta"]
    );
    check!(
        vec![
            f("cp:vip", Op::IsTrue, &[]),
            f("cp:region", Op::Equals, &["US"])
        ],
        [] as [&str; 0]
    );

    // Native columns.
    check!(vec![f("currency", Op::Equals, &["USD"])], ["Beta"]);
    check!(vec![f("currency", Op::NotEquals, &["EUR"])], ["Beta"]);
    check!(vec![f("currency", Op::In, &["EUR"])], ["Acme", "Gamma"]);
    check!(
        vec![f("currency", Op::In, &["EUR", "USD"])],
        ["Acme", "Beta", "Gamma"]
    );
    check!(
        vec![f("created_at", Op::After, &["2000-01-01"])],
        ["Acme", "Beta", "Gamma"]
    );
    check!(
        vec![f("created_at", Op::Before, &["2000-01-01"])],
        [] as [&str; 0]
    );
    let tomorrow = (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339();
    check!(
        vec![f(
            "created_at",
            Op::Between,
            &["2000-01-01", tomorrow.as_str()]
        )],
        ["Acme", "Beta", "Gamma"]
    );
    check!(
        vec![
            f("created_at", Op::After, &["2000-01-01T00:00:00Z"]),
            f("currency", Op::Equals, &["EUR"]),
            f("cp:tier", Op::IsSet, &[])
        ],
        ["Acme"]
    );

    // Built-in fields: billing country, invoice credit balance (major units, scaled by currency),
    // billing email.
    {
        use diesel_models::schema::customer::dsl as c;
        let mut conn = env.conn().await;
        for (name, address, balance, email) in [
            (
                "Acme",
                Some(json!({ "country": "FR" })),
                150_000_i64,
                Some("a@acme.test"),
            ),
            ("Beta", Some(json!({ "country": "US" })), 0, Some("")),
            ("Gamma", None, 500, None),
        ] {
            let id: common_domain::ids::CustomerId = diesel::update(
                c::customer
                    .filter(c::tenant_id.eq(tenant))
                    .filter(c::name.eq(name)),
            )
            .set((c::billing_address.eq(address), c::billing_email.eq(email)))
            .returning(c::id)
            .get_result(&mut conn)
            .await
            .unwrap();
            // The balance is the invoice credit spendable now, from the credit ledger.
            if balance > 0 {
                crate::helpers::credit::fund(env.store(), tenant, id, None, balance).await;
            }
        }
    }
    check!(vec![f("country", Op::Equals, &["FR"])], ["Acme"]);
    check!(vec![f("country", Op::Equals, &["fr"])], ["Acme"]);
    check!(vec![f("country", Op::In, &["FR", "US"])], ["Acme", "Beta"]);
    check!(vec![f("country", Op::NotEquals, &["FR"])], ["Beta"]);
    check!(vec![f("country", Op::IsEmpty, &[])], ["Gamma"]);
    check!(vec![f("balance", Op::Gt, &["1000"])], ["Acme"]);
    check!(vec![f("balance", Op::Equals, &["1500"])], ["Acme"]);
    check!(vec![f("balance", Op::Lt, &["1"])], ["Beta"]);
    check!(vec![f("balance", Op::Between, &["1", "10"])], ["Gamma"]);
    check!(vec![f("billing_email", Op::IsSet, &[])], ["Acme"]);
    check!(
        vec![f("billing_email", Op::IsEmpty, &[])],
        ["Beta", "Gamma"]
    );
    assert_invalid(list(vec![f("country", Op::Equals, &["ZZ"])]).await);
    assert_invalid(list(vec![f("balance", Op::Gt, &["lots"])]).await);
    assert_invalid(list(vec![f("billing_email", Op::Equals, &["a@acme.test"])]).await);
    assert_invalid(list(vec![f("currency", Op::Equals, &["ZZZ"])]).await);

    // Rejections.
    assert_invalid(list(vec![f("cp:nope", Op::IsSet, &[])]).await);
    assert_invalid(list(vec![f("cp:tier", Op::Gt, &["a"])]).await);
    assert_invalid(list(vec![f("cp:vip", Op::Equals, &["true"])]).await);
    assert_invalid(list(vec![f("cp:signed", Op::In, &["2026-01-15"])]).await);
    assert_invalid(list(vec![f("cp:seats", Op::Gt, &["many"])]).await);
    assert_invalid(list(vec![f("cp:signed", Op::Before, &["15/01/2026"])]).await);
    assert_invalid(list(vec![f("cp:seats", Op::Between, &["1"])]).await);
    assert_invalid(list(vec![f("cp:tier", Op::In, &[])]).await);
    assert_invalid(list(vec![f("name", Op::Equals, &["Acme"])]).await);
    assert_invalid(list(vec![f("currency", Op::Gt, &["EUR"])]).await);
    assert_invalid(list(vec![f("created_at", Op::Equals, &["2026-01-01"])]).await);
    assert_invalid(list(vec![f("created_at", Op::Before, &["yesterday"])]).await);
    // One bad filter fails the whole request.
    assert_invalid(
        list(vec![
            f("cp:tier", Op::IsSet, &[]),
            f("cp:nope", Op::IsSet, &[]),
        ])
        .await,
    );

    // Archived definitions stop being filterable.
    check!(vec![f("cp:legacy", Op::IsSet, &[])], ["Acme"]);
    archive_custom_property_definition(store, tenant, legacy.id).await;
    assert_invalid(list(vec![f("cp:legacy", Op::IsSet, &[])]).await);

    // Definitions are tenant-scoped: the seeded tenant doesn't know this test's keys. (Not `tier`:
    // other test files define that one on the seeded tenant, and definitions are cached
    // in-process.)
    assert_invalid(
        store
            .list_customers_with_metrics(
                TENANT_ID,
                page(),
                None,
                None,
                None,
                vec![],
                vec![],
                vec![f("cp:last_seen", Op::IsSet, &[])],
            )
            .await,
    );
}

#[rstest]
#[tokio::test]
async fn test_subscription_list_filters(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    type T = CustomPropertyType;
    const S: CustomPropertyEntityType = CustomPropertyEntityType::Subscription;

    let s1 = subscription().create(env.services()).await;
    let s2 = subscription().create(env.services()).await;
    let s3 = subscription().create(env.services()).await;

    define(&env, TENANT_ID, S, "po_ref", T::Text, &[]).await;
    define(&env, TENANT_ID, S, "priority", T::Number, &[]).await;
    define(&env, TENANT_ID, S, "renewal", T::Date, &[]).await;

    {
        use diesel_models::schema::subscription::dsl as s;
        let mut conn = env.conn().await;
        for (id, props) in [
            (
                s1,
                json!({ "po_ref": "PO-1001", "priority": 1, "renewal": "2026-06-30" }),
            ),
            (
                s2,
                json!({ "po_ref": "po-2002", "priority": 5, "renewal": "2027-01-31" }),
            ),
            (s3, json!({})),
        ] {
            diesel::update(s::subscription.filter(s::id.eq(id)))
                .set(s::custom_properties.eq(props))
                .execute(&mut conn)
                .await
                .unwrap();
        }
    }

    let list = |filters: Vec<Filter>| async move {
        store
            .list_subscriptions(TENANT_ID, None, None, None, None, filters, None, page())
            .await
            .map(|p| {
                p.items
                    .into_iter()
                    .map(|s| s.id.as_uuid())
                    .collect::<BTreeSet<_>>()
            })
    };
    macro_rules! check {
        ($filters:expr, $expected:expr) => {
            assert_eq!(
                list($filters).await.expect("list subscriptions"),
                $expected
                    .into_iter()
                    .map(|id| id.as_uuid())
                    .collect::<BTreeSet<_>>(),
                "filters: {:?}",
                $filters
            );
        };
    }

    check!(Vec::<Filter>::new(), [s1, s2, s3]);
    check!(vec![f("cp:po_ref", Op::Equals, &["PO-1001"])], [s1]);
    check!(vec![f("cp:po_ref", Op::Contains, &["po-"])], [s1, s2]);
    check!(vec![f("cp:po_ref", Op::In, &["po-2002", "x"])], [s2]);
    check!(vec![f("cp:po_ref", Op::IsSet, &[])], [s1, s2]);
    check!(vec![f("cp:po_ref", Op::IsEmpty, &[])], [s3]);
    check!(vec![f("cp:priority", Op::Gt, &["1"])], [s2]);
    check!(vec![f("cp:priority", Op::Between, &["1", "5"])], [s1, s2]);
    check!(vec![f("cp:renewal", Op::Before, &["2027-01-01"])], [s1]);
    check!(vec![f("cp:renewal", Op::After, &["2026-06-30"])], [s2]);
    check!(
        vec![
            f("cp:priority", Op::Gte, &["1"]),
            f("cp:renewal", Op::After, &["2027-01-01"])
        ],
        [s2]
    );

    // Filters compose with the existing list arguments.
    let for_customer = store
        .list_subscriptions(
            TENANT_ID,
            Some(CUST_UBER_ID),
            None,
            None,
            None,
            vec![f("cp:priority", Op::Lt, &["5"])],
            None,
            page(),
        )
        .await
        .unwrap();
    assert_eq!(
        for_customer
            .items
            .into_iter()
            .map(|s| s.id)
            .collect::<Vec<_>>(),
        vec![s1]
    );

    // Built-in fields. MRR is in major units scaled by currency: ¥5000 is 5000, not 50.00.
    {
        use diesel_models::schema::subscription::dsl as s;
        let mut conn = env.conn().await;
        for (id, currency, mrr, start, end) in [
            (s1, "EUR", 1_000_i64, "2026-01-15", None),
            (s2, "USD", 250_000, "2026-03-01", None),
            (s3, "JPY", 5_000, "2026-03-02", Some("2026-12-31")),
        ] {
            diesel::update(s::subscription.filter(s::id.eq(id)))
                .set((
                    s::currency.eq(currency),
                    s::mrr_cents.eq(mrr),
                    s::start_date.eq(chrono::NaiveDate::from_str(start).unwrap()),
                    s::end_date.eq(end.map(|e| chrono::NaiveDate::from_str(e).unwrap())),
                ))
                .execute(&mut conn)
                .await
                .unwrap();
        }
    }
    check!(vec![f("currency", Op::In, &["EUR", "USD"])], [s1, s2]);
    check!(vec![f("currency", Op::NotEquals, &["JPY"])], [s1, s2]);
    check!(vec![f("mrr", Op::Equals, &["10"])], [s1]);
    check!(vec![f("mrr", Op::Gt, &["20"])], [s2, s3]);
    check!(vec![f("mrr", Op::Between, &["5", "100"])], [s1]);
    check!(vec![f("start_date", Op::Before, &["2026-03-01"])], [s1]);
    check!(vec![f("start_date", Op::After, &["2026-03-01"])], [s3]);
    check!(
        vec![f("start_date", Op::Between, &["2026-03-01", "2026-03-02"])],
        [s2, s3]
    );
    check!(vec![f("end_date", Op::IsSet, &[])], [s3]);
    check!(vec![f("end_date", Op::IsEmpty, &[])], [s1, s2]);

    // Customer / plan references match what the dedicated list arguments return.
    let uber_subs: BTreeSet<_> = store
        .list_subscriptions(
            TENANT_ID,
            Some(CUST_UBER_ID),
            None,
            None,
            None,
            vec![],
            None,
            page(),
        )
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|s| s.id.as_uuid())
        .filter(|id| [s1, s2, s3].iter().any(|s| s.as_uuid() == *id))
        .collect();
    assert_eq!(
        list(vec![f(
            "customer",
            Op::Equals,
            &[&CUST_UBER_ID.as_base62()]
        )])
        .await
        .unwrap(),
        uber_subs
    );
    let plan_of_s1: uuid::Uuid = {
        use diesel_models::schema::plan_version::dsl as pv;
        use diesel_models::schema::subscription::dsl as s;
        let mut conn = env.conn().await;
        let version: uuid::Uuid = s::subscription
            .filter(s::id.eq(s1))
            .select(s::plan_version_id)
            .first(&mut conn)
            .await
            .unwrap();
        pv::plan_version
            .filter(pv::id.eq(version))
            .select(pv::plan_id)
            .first(&mut conn)
            .await
            .unwrap()
    };
    let plan_filter = common_domain::ids::PlanId::from(plan_of_s1).as_base62();
    assert!(
        list(vec![f("plan", Op::In, &[&plan_filter])])
            .await
            .unwrap()
            .contains(&s1.as_uuid())
    );
    check!(
        vec![f(
            "plan",
            Op::Equals,
            &[&common_domain::ids::PlanId::new().as_base62()]
        )],
        [] as [common_domain::ids::SubscriptionId; 0]
    );

    // Unknown fields / bad operators and values are rejected.
    assert_invalid(list(vec![f("status", Op::Equals, &["ACTIVE"])]).await);
    assert_invalid(list(vec![f("created_at", Op::After, &["2000-01-01"])]).await);
    assert_invalid(list(vec![f("currency", Op::Equals, &["ZZZ"])]).await);
    assert_invalid(list(vec![f("mrr", Op::Gt, &["abc"])]).await);
    assert_invalid(list(vec![f("start_date", Op::Gt, &["2026-01-01"])]).await);
    assert_invalid(list(vec![f("plan", Op::In, &["not-an-id"])]).await);
    assert_invalid(list(vec![f("customer", Op::Equals, &["plan_123"])]).await);
    assert_invalid(list(vec![f("cp:nope", Op::IsSet, &[])]).await);
    assert_invalid(list(vec![f("cp:po_ref", Op::Between, &["a", "b"])]).await);
    assert_invalid(list(vec![f("cp:priority", Op::Contains, &["1"])]).await);
}

#[rstest]
#[tokio::test]
async fn test_invoice_list_filters(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.skip_credit_invariants("inserts documents with arbitrary totals to exercise list filters");
    let store = env.store();
    type T = CustomPropertyType;
    const I: CustomPropertyEntityType = CustomPropertyEntityType::Invoice;

    // Each OnStart subscription issues its first invoice on creation.
    let mut invoices = vec![];
    for _ in 0..3 {
        let sub = subscription().create(env.services()).await;
        let inv = env.get_invoices(sub).await;
        assert_eq!(inv.len(), 1, "expected one invoice per new subscription");
        invoices.push(inv[0].id);
    }
    let (i1, i2, i3) = (invoices[0], invoices[1], invoices[2]);

    define(&env, TENANT_ID, I, "cost_center", T::Text, &[]).await;
    define(&env, TENANT_ID, I, "approved_on", T::Date, &[]).await;
    define(&env, TENANT_ID, I, "po_amount", T::Number, &[]).await;

    {
        use diesel_models::schema::invoice::dsl as i;
        let mut conn = env.conn().await;
        for (id, props) in [
            (
                i1,
                json!({ "cost_center": "R&D", "approved_on": "2026-02-01", "po_amount": 100.5 }),
            ),
            (
                i2,
                json!({ "cost_center": "Sales", "approved_on": "2026-03-15", "po_amount": 20 }),
            ),
            (i3, json!({})),
        ] {
            diesel::update(i::invoice.filter(i::id.eq(id)))
                .set(i::custom_properties.eq(props))
                .execute(&mut conn)
                .await
                .unwrap();
        }
    }

    let list = |filters: Vec<Filter>| async move {
        store
            .list_invoices(
                TENANT_ID,
                None,
                None,
                None,
                None,
                None,
                false,
                None,
                filters,
                None,
                page(),
            )
            .await
            .map(|p| {
                p.items
                    .into_iter()
                    .map(|i| i.invoice.id.as_uuid())
                    .collect::<BTreeSet<_>>()
            })
    };
    macro_rules! check {
        ($filters:expr, $expected:expr) => {
            assert_eq!(
                list($filters).await.expect("list invoices"),
                $expected
                    .into_iter()
                    .map(|id| id.as_uuid())
                    .collect::<BTreeSet<_>>(),
                "filters: {:?}",
                $filters
            );
        };
    }

    check!(Vec::<Filter>::new(), [i1, i2, i3]);
    check!(vec![f("cp:cost_center", Op::Equals, &["R&D"])], [i1]);
    check!(
        vec![f("cp:cost_center", Op::In, &["R&D", "Sales"])],
        [i1, i2]
    );
    check!(vec![f("cp:cost_center", Op::Contains, &["ale"])], [i2]);
    check!(vec![f("cp:cost_center", Op::IsEmpty, &[])], [i3]);
    check!(vec![f("cp:approved_on", Op::Before, &["2026-03-01"])], [i1]);
    check!(vec![f("cp:approved_on", Op::After, &["2026-03-01"])], [i2]);
    check!(
        vec![f(
            "cp:approved_on",
            Op::Between,
            &["2026-01-01", "2026-12-31"]
        )],
        [i1, i2]
    );
    check!(vec![f("cp:approved_on", Op::IsSet, &[])], [i1, i2]);
    check!(vec![f("cp:po_amount", Op::Gt, &["100"])], [i1]);
    check!(vec![f("cp:po_amount", Op::Equals, &["20"])], [i2]);

    // Built-in fields. Amounts are major units scaled by currency (¥50 > 1).
    {
        use diesel_models::schema::invoice::dsl as i;
        let mut conn = env.conn().await;
        let at = |s: &str| chrono::NaiveDateTime::from_str(s).unwrap();
        for (id, currency, total, due, invoice_date, due_at) in [
            (
                i1,
                "EUR",
                10_050_i64,
                0_i64,
                "2026-01-10",
                Some(at("2026-02-10T10:00:00")),
            ),
            (i2, "USD", 200_000, 200_000, "2026-02-10", None),
            (i3, "JPY", 50, 50, "2026-03-10", None),
        ] {
            diesel::update(i::invoice.filter(i::id.eq(id)))
                .set((
                    i::currency.eq(currency),
                    i::total.eq(total),
                    i::amount_due.eq(due),
                    i::invoice_date.eq(chrono::NaiveDate::from_str(invoice_date).unwrap()),
                    i::due_at.eq(due_at),
                ))
                .execute(&mut conn)
                .await
                .unwrap();
        }
    }
    check!(vec![f("currency", Op::Equals, &["EUR"])], [i1]);
    check!(vec![f("total", Op::Equals, &["100.5"])], [i1]);
    check!(vec![f("total", Op::Gt, &["1"])], [i1, i2, i3]);
    check!(vec![f("total", Op::Lt, &["60"])], [i3]);
    check!(vec![f("amount_due", Op::Equals, &["0"])], [i1]);
    check!(vec![f("amount_due", Op::Gte, &["2000"])], [i2]);
    check!(vec![f("invoice_date", Op::Before, &["2026-02-10"])], [i1]);
    check!(
        vec![f(
            "invoice_date",
            Op::Between,
            &["2026-02-10", "2026-03-10"]
        )],
        [i2, i3]
    );
    // A date-only value covers the whole day on a timestamp column.
    check!(
        vec![f("due_at", Op::Before, &["2026-02-10"])],
        [] as [common_domain::ids::InvoiceId; 0]
    );
    check!(vec![f("due_at", Op::After, &["2026-02-09"])], [i1]);
    check!(
        vec![f("due_at", Op::Between, &["2026-02-10", "2026-02-10"])],
        [i1]
    );
    check!(vec![f("due_at", Op::IsEmpty, &[])], [i2, i3]);
    check!(
        vec![
            f("currency", Op::In, &["EUR", "JPY"]),
            f("amount_due", Op::Gt, &["10"])
        ],
        [i3]
    );
    assert_invalid(list(vec![f("status", Op::Equals, &["DRAFT"])]).await);
    assert_invalid(list(vec![f("total", Op::Contains, &["1"])]).await);
    assert_invalid(list(vec![f("due_at", Op::Before, &["soon"])]).await);
    assert_invalid(list(vec![f("cp:nope", Op::IsSet, &[])]).await);
    assert_invalid(list(vec![f("cp:approved_on", Op::Gt, &["2026-01-01"])]).await);
    assert_invalid(list(vec![f("cp:approved_on", Op::Between, &["2026-01-01"])]).await);
}

#[rstest]
#[tokio::test]
async fn test_credit_note_list_filters(#[future] test_env: TestEnv) {
    let env = test_env.await;
    env.skip_credit_invariants("inserts documents with arbitrary totals to exercise list filters");
    let store = env.store();
    type T = CustomPropertyType;
    const CN: CustomPropertyEntityType = CustomPropertyEntityType::CreditNote;

    define(
        &env,
        TENANT_ID,
        CN,
        "reason_code",
        T::SingleSelect,
        &["goodwill", "billing_error"],
    )
    .await;
    define(&env, TENANT_ID, CN, "ticket", T::Text, &[]).await;
    define(&env, TENANT_ID, CN, "approved_on", T::Date, &[]).await;

    // One full debt-cancellation draft per finalized invoice, with its properties set through the
    // real create path (validated against the definitions above).
    let mut notes = vec![];
    for props in [
        json!({ "reason_code": "goodwill", "ticket": "SUP-101", "approved_on": "2026-02-01" }),
        json!({ "reason_code": "billing_error", "ticket": "SUP-202" }),
        json!({}),
    ] {
        let sub = subscription().create(env.services()).await;
        let invoice = env.get_invoices(sub).await.remove(0);
        if invoice.status != InvoiceStatusEnum::Finalized {
            env.manual_finalize_invoice(invoice.id)
                .await
                .expect("finalize invoice");
        }
        let note = store
            .create_credit_note(
                Actor::System,
                TENANT_ID,
                CreateCreditNoteParams {
                    invoice_id: invoice.id,
                    line_items: vec![],
                    reason: None,
                    memo: None,
                    credit_type: CreditType::DebtCancellation,
                    custom_properties: Some(props),
                },
            )
            .await
            .expect("create credit note");
        notes.push(note.id);
    }
    let (c1, c2, c3) = (notes[0], notes[1], notes[2]);

    let list = |filters: Vec<Filter>| async move {
        store
            .list_credit_notes(TENANT_ID, None, None, None, None, filters, None, page())
            .await
            .map(|p| {
                p.items
                    .into_iter()
                    .map(|c| c.id.as_uuid())
                    .collect::<BTreeSet<_>>()
            })
    };
    macro_rules! check {
        ($filters:expr, $expected:expr) => {
            assert_eq!(
                list($filters).await.expect("list credit notes"),
                $expected
                    .into_iter()
                    .map(|id| id.as_uuid())
                    .collect::<BTreeSet<_>>(),
                "filters: {:?}",
                $filters
            );
        };
    }

    check!(Vec::<Filter>::new(), [c1, c2, c3]);
    check!(vec![f("cp:reason_code", Op::Equals, &["goodwill"])], [c1]);
    check!(
        vec![f("cp:reason_code", Op::In, &["goodwill", "billing_error"])],
        [c1, c2]
    );
    check!(vec![f("cp:ticket", Op::Contains, &["202"])], [c2]);
    check!(vec![f("cp:ticket", Op::IsEmpty, &[])], [c3]);
    check!(vec![f("cp:approved_on", Op::Before, &["2026-03-01"])], [c1]);
    // AND-combined.
    check!(
        vec![
            f("cp:reason_code", Op::IsSet, &[]),
            f("cp:ticket", Op::Contains, &["SUP-1"]),
        ],
        [c1]
    );

    // Built-in fields: totals are stored negative and filtered as positive amounts.
    {
        use diesel::sql_types::{BigInt, Uuid as SqlUuid};
        let mut conn = env.conn().await;
        for (id, total) in [(c1, -1_000_i64), (c2, -25_000), (c3, -500)] {
            // The cancelled debt follows the rewritten total: it never exceeds what was credited.
            diesel::sql_query(
                "UPDATE credit_note SET total = $1,
                     debt_cancelled_amount_cents = CASE WHEN credit_type = 'DEBT_CANCELLATION' THEN abs($1)
                                                        ELSE least(debt_cancelled_amount_cents, abs($1)) END
                 WHERE id = $2",
            )
            .bind::<BigInt, _>(total)
            .bind::<SqlUuid, _>(id.as_uuid())
            .execute(&mut conn)
            .await
            .unwrap();
        }
    }
    check!(vec![f("total", Op::Gt, &["9"])], [c1, c2]);
    check!(vec![f("total", Op::Between, &["5", "10"])], [c1, c3]);
    check!(
        vec![f("credit_type", Op::Equals, &["DEBT_CANCELLATION"])],
        [c1, c2, c3]
    );
    check!(
        vec![f("credit_type", Op::In, &["REFUND", "CREDIT_TO_BALANCE"])],
        [] as [common_domain::ids::CreditNoteId; 0]
    );
    check!(
        vec![f("created_at", Op::After, &["2000-01-01"])],
        [c1, c2, c3]
    );
    assert_invalid(list(vec![f("credit_type", Op::Equals, &["GIFT"])]).await);

    assert_invalid(list(vec![f("status", Op::Equals, &["DRAFT"])]).await);
    assert_invalid(list(vec![f("cp:nope", Op::IsSet, &[])]).await);
    assert_invalid(list(vec![f("cp:reason_code", Op::Contains, &["good"])]).await);
}
