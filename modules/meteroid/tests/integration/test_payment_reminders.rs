//! Payment reminder sequences: default seeding, validation, audiences, archive and the
//! per-customer override (spec §7.2 group B).

use std::collections::{HashMap, HashSet};

use chrono::Utc;
use common_domain::country::CountryCode;
use common_domain::ids::{BaseId, CustomerId, InvoiceId, InvoicingEntityId, SubscriptionId};
use diesel_models::connectors::ConnectorRow;
use diesel_models::customer_payment_methods::CustomerPaymentMethodRow;
use diesel_models::entity_activity::EntityActivityRow;
use diesel_models::enums::{ConnectorStatusEnum, PaymentMethodTypeEnum, ReminderStopReasonEnum};
use diesel_models::payment_reminders::PaymentReminderSequenceRow;
use rstest::rstest;
use serde_json::json;

use meteroid_store::domain::custom_property_definitions::{
    CustomPropertyDefinitionNew, PropertyConfig, SelectOption,
};
use meteroid_store::domain::customer_payment_methods::ResolvedPaymentMethod;
use meteroid_store::domain::customers::Address;
use meteroid_store::domain::entity_activity::Actor;
use meteroid_store::domain::enums::{
    CustomPropertyEntityType, CustomPropertyType, CustomerType, InvoiceType, TenantEnvironmentEnum,
};
use meteroid_store::domain::payment_reminders::{
    CustomerReminderPreference, EmailCopy, EmailCopyByLocale, PaymentReminderSequencePatch,
    PaymentReminderStepInput,
};
use meteroid_store::domain::subscriptions::PaymentMethodsConfig;
use meteroid_store::domain::tenants::TenantNew;
use meteroid_store::domain::{CustomerNew, CustomerPatch, CustomerTaxStatus, InvoicingEntityNew};
use meteroid_store::errors::StoreError;
use meteroid_store::repositories::customer_payment_methods::CustomerPaymentMethodsInterface;
use meteroid_store::repositories::invoicing_entities::{
    InvoicingEntityInterface, InvoicingEntityInterfaceAuto,
};
use meteroid_store::repositories::payment_reminders::DEFAULT_SEQUENCE_NAME;
use meteroid_store::repositories::{
    CustomersInterface, PaymentRemindersInterface, TenantInterface,
};
use meteroid_testkit::custom_properties::create_custom_property_definition;

use crate::data::ids::*;
use crate::harness::reminders::{
    ENTITY_LOCALE, FinalizedInvoice, audience, copies, copy, match_all, steps,
};
use crate::harness::{TestEnv, subscription, test_env};

const ACTOR: Actor = Actor::User { id: USER_ID };

fn assert_invalid_argument<T: std::fmt::Debug>(
    result: Result<T, error_stack::Report<StoreError>>,
    label: &str,
) {
    match result {
        Err(err) => assert!(
            matches!(err.current_context(), StoreError::InvalidArgument(_)),
            "{label}: expected InvalidArgument, got {err:?}"
        ),
        Ok(v) => panic!("{label}: expected InvalidArgument, got {v:?}"),
    }
}

async fn activities(env: &TestEnv, entity_type: &str, id: uuid::Uuid) -> Vec<EntityActivityRow> {
    let mut conn = env.conn().await;
    EntityActivityRow::list_by_entity(&mut conn, TENANT_ID, entity_type, id, None, 50)
        .await
        .expect("list activities")
}

fn changed_fields(row: &EntityActivityRow) -> Vec<String> {
    row.metadata
        .as_ref()
        .and_then(|m| m.get("changes"))
        .and_then(|c| c.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| c.get("field").and_then(|f| f.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn customer_patch(id: CustomerId) -> CustomerPatch {
    CustomerPatch {
        id,
        name: None,
        alias: None,
        billing_email: None,
        invoicing_emails: None,
        phone: None,
        currency: None,
        billing_address: None,
        shipping_address: None,
        invoicing_entity_id: None,
        vat_number: None,
        legal_number: None,
        buyer_reference: None,
        custom_taxes: None,
        current_payment_method_id: None,
        tax_status: None,
        exemption_reason: None,
        connected_account_id: None,
        custom_properties: None,
        preferred_locales: None,
        customer_type: None,
        first_name: None,
        last_name: None,
    }
}

async fn insert_customer(
    env: &TestEnv,
    name: &str,
    country: Option<&str>,
    custom_properties: serde_json::Value,
) -> CustomerId {
    env.store()
        .insert_customer(
            ACTOR,
            CustomerNew {
                name: name.to_string(),
                customer_type: CustomerType::Company,
                first_name: None,
                last_name: None,
                alias: None,
                billing_email: None,
                invoicing_emails: vec![format!("{}@example.test", name.to_lowercase())],
                phone: None,
                currency: "EUR".to_string(),
                billing_address: country.map(|code| Address {
                    line1: None,
                    line2: None,
                    city: None,
                    country: Some(CountryCode {
                        code: code.to_string(),
                        name: code.to_string(),
                    }),
                    state: None,
                    zip_code: None,
                }),
                shipping_address: None,
                invoicing_entity_id: None,
                force_created_date: None,
                tax_status: CustomerTaxStatus::Taxable,
                exemption_reason: None,
                vat_number: None,
                legal_number: None,
                buyer_reference: None,
                custom_taxes: vec![],
                connected_account_id: None,
                custom_properties,
                preferred_locales: vec![],
            },
            TENANT_ID,
        )
        .await
        .expect("insert customer")
        .id
}

async fn define_property(
    env: &TestEnv,
    key: &str,
    property_type: CustomPropertyType,
    options: &[&str],
) {
    let config = PropertyConfig {
        options: (!options.is_empty()).then(|| {
            options
                .iter()
                .map(|v| SelectOption {
                    value: v.to_string(),
                    label: None,
                })
                .collect()
        }),
        ..Default::default()
    };
    create_custom_property_definition(
        env.store(),
        CustomPropertyDefinitionNew {
            tenant_id: TENANT_ID,
            entity_type: CustomPropertyEntityType::Customer,
            key: key.to_string(),
            name: key.to_string(),
            description: None,
            property_type,
            config,
            required: false,
            default_value: None,
            display_order: 0,
        },
    )
    .await;
}

async fn create_second_entity(env: &TestEnv) -> InvoicingEntityId {
    env.store()
        .create_invoicing_entity(
            InvoicingEntityNew {
                legal_name: Some("Second Entity".to_string()),
                ..Default::default()
            },
            TENANT_ID,
            ORGANIZATION_ID,
        )
        .await
        .expect("create invoicing entity")
        .id
}

#[rstest]
#[tokio::test]
async fn default_sequence_is_created_disabled_on_built_in_copy_idempotently(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;

    // The seeded entity predates the feature: the first listing seeds its default lazily.
    let listed = env.list_sequences(INVOICING_ENTITY_ID).await;
    assert_eq!(listed.len(), 1);
    let default = &listed[0].sequence;
    assert!(default.is_default());
    assert!(!default.enabled);
    assert_eq!(default.name, DEFAULT_SEQUENCE_NAME);
    assert_eq!(listed[0].in_flight_invoices, 0);
    let offsets: Vec<i32> = default.steps.iter().map(|s| s.offset_days).collect();
    assert_eq!(offsets, vec![-3, 0, 7, 14]);
    assert!(default.steps.iter().all(|s| s.content.0.is_empty()));

    // Listing again seeds nothing new and no audit row was written for the system seed.
    let again = env.list_sequences(INVOICING_ENTITY_ID).await;
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].sequence.id, default.id);
    assert!(
        activities(&env, "payment_reminder_sequence", default.id.as_uuid())
            .await
            .is_empty()
    );

    // A newly created entity gets its default at creation, before any listing.
    let second = create_second_entity(&env).await;
    let mut conn = env.conn().await;
    let rows = PaymentReminderSequenceRow::list_by_entity(&mut conn, TENANT_ID, second, true)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].audience.is_none() && !rows[0].enabled);
}

#[rstest]
#[tokio::test]
async fn sequence_validation_rejects_bad_steps_copy_locales_and_markdown(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let entity = INVOICING_ENTITY_ID;
    let store = env.store();

    let create = |name: &str, steps: Vec<PaymentReminderStepInput>| {
        let name = name.to_string();
        async move {
            store
                .create_reminder_sequence(
                    ACTOR,
                    TENANT_ID,
                    meteroid_store::domain::payment_reminders::PaymentReminderSequenceNew {
                        invoicing_entity_id: entity,
                        name,
                        enabled: true,
                        audience: Some(match_all()),
                        steps,
                    },
                )
                .await
        }
    };

    assert_invalid_argument(create("", steps(&[0])).await, "blank name");
    assert_invalid_argument(create(&"x".repeat(81), steps(&[0])).await, "long name");
    assert_invalid_argument(create("s", steps(&[0, 7, 7])).await, "non-increasing");
    assert_invalid_argument(create("s", steps(&[7, 0])).await, "decreasing");
    assert_invalid_argument(create("s", steps(&[-31])).await, "below range");
    assert_invalid_argument(create("s", steps(&[366])).await, "above range");
    let twenty_one: Vec<i32> = (0..21).collect();
    assert_invalid_argument(create("s", steps(&twenty_one)).await, "21 steps");

    let step = |content: EmailCopyByLocale| {
        vec![PaymentReminderStepInput {
            offset_days: 0,
            content,
        }]
    };
    assert_invalid_argument(
        create("s", step(copy("nl-BE", "Subject"))).await,
        "nl-BE key",
    );
    assert_invalid_argument(
        create("s", step(copy("en-US", "Subject"))).await,
        "missing entity-locale entry",
    );
    let mut blank = copies("Subject", "Body {{invoice_number}}");
    blank.0.get_mut(ENTITY_LOCALE).unwrap().subject = "  ".to_string();
    assert_invalid_argument(create("s", step(blank)).await, "blank entity-locale entry");
    assert_invalid_argument(
        create("s", step(copies("Hello {{unknown}}", "Body"))).await,
        "unknown variable",
    );
    for (label, body) in [
        ("link", "Pay [here](https://example.com)"),
        ("heading", "# Pay now"),
        ("html", "Pay <b>now</b>"),
    ] {
        assert_invalid_argument(create("s", step(copies("Subject", body))).await, label);
    }
    assert_invalid_argument(
        create("s", step(copies(&"s".repeat(201), "Body"))).await,
        "subject over 200 chars",
    );
    assert_invalid_argument(
        create("s", step(copies("Subject", &"b".repeat(5001)))).await,
        "body over 5000 chars",
    );
    let mut english_only_extra = copies("Subject", "Body");
    english_only_extra.0.insert(
        "de-DE".to_string(),
        EmailCopy {
            subject: "Betreff".to_string(),
            body: "Text".to_string(),
        },
    );
    let three = create("Three locales", step(english_only_extra))
        .await
        .expect("extra supported locales are fine");

    // The zero-step sequence is the documented way to send nothing to an audience.
    let created = create("Silent", vec![]).await.expect("zero steps allowed");
    assert!(created.sequence.steps.is_empty() && created.sequence.enabled);

    // The default cannot receive an audience; a non-default cannot lose it.
    let default = env
        .list_sequences(entity)
        .await
        .into_iter()
        .find(|s| s.sequence.is_default())
        .unwrap();
    let now = Utc::now().naive_utc();
    assert_invalid_argument(
        env.store()
            .update_reminder_sequence(
                ACTOR,
                TENANT_ID,
                PaymentReminderSequencePatch {
                    id: default.sequence.id,
                    audience: Some(Some(match_all())),
                    ..Default::default()
                },
                now,
            )
            .await,
        "audience on default",
    );
    assert_invalid_argument(
        env.store()
            .update_reminder_sequence(
                ACTOR,
                TENANT_ID,
                PaymentReminderSequencePatch {
                    id: created.sequence.id,
                    audience: Some(None),
                    ..Default::default()
                },
                now,
            )
            .await,
        "clearing a non-default audience",
    );

    // A valid update replaces the steps wholesale and is audited.
    let updated = env
        .store()
        .update_reminder_sequence(
            ACTOR,
            TENANT_ID,
            PaymentReminderSequencePatch {
                id: created.sequence.id,
                name: Some("Renamed".to_string()),
                steps: Some(steps(&[-3, 5])),
                ..Default::default()
            },
            now,
        )
        .await
        .expect("update");
    assert_eq!(updated.sequence.name, "Renamed");
    let offsets: Vec<i32> = updated
        .sequence
        .steps
        .iter()
        .map(|s| s.offset_days)
        .collect();
    assert_eq!(offsets, vec![-3, 5]);
    let rows = activities(
        &env,
        "payment_reminder_sequence",
        created.sequence.id.as_uuid(),
    )
    .await;
    let types: Vec<&str> = rows.iter().map(|r| r.activity_type.as_str()).collect();
    assert!(types.contains(&"reminder_sequence.created"));
    let update_row = rows
        .iter()
        .find(|r| r.activity_type == "reminder_sequence.updated")
        .expect("update audit");
    let fields = changed_fields(update_row);
    assert!(fields.contains(&"name".to_string()) && fields.contains(&"steps".to_string()));

    // Created sequences land just before the default, which keeps the highest position.
    let listed = env.list_sequences(entity).await;
    let positions: Vec<(bool, i32)> = listed
        .iter()
        .map(|s| (s.sequence.is_default(), s.sequence.position))
        .collect();
    assert_eq!(positions, vec![(false, 0), (false, 1), (true, 2)]);

    // Reorder: two-pass under the partial unique index, default forced last.
    let second = env.create_sequence(entity, &[0], true).await;
    let reordered = env
        .store()
        .reorder_reminder_sequences(
            ACTOR,
            TENANT_ID,
            entity,
            vec![second.id, created.sequence.id, three.sequence.id],
        )
        .await
        .expect("reorder");
    let order: Vec<_> = reordered.iter().map(|s| s.sequence.id).collect();
    assert_eq!(
        order,
        vec![
            second.id,
            created.sequence.id,
            three.sequence.id,
            default.sequence.id
        ]
    );
    let positions: Vec<i32> = reordered.iter().map(|s| s.sequence.position).collect();
    assert_eq!(positions, vec![0, 1, 2, 3]);
    assert_invalid_argument(
        env.store()
            .reorder_reminder_sequences(ACTOR, TENANT_ID, entity, vec![second.id])
            .await,
        "partial reorder",
    );
}

#[rstest]
#[tokio::test]
async fn audience_rejects_segment_unknown_property_and_bad_literals(#[future] test_env: TestEnv) {
    let env = test_env.await;
    define_property(
        &env,
        "tier",
        CustomPropertyType::SingleSelect,
        &["enterprise", "smb"],
    )
    .await;
    define_property(&env, "seats", CustomPropertyType::Number, &[]).await;
    define_property(&env, "vip", CustomPropertyType::Boolean, &[]).await;
    define_property(&env, "signed_at", CustomPropertyType::Date, &[]).await;

    let cases = vec![
        (
            "eq bool on a number property",
            json!([{ "property": { "key": "seats", "op": "eq", "value": true } }]),
        ),
        (
            "eq number on a boolean property",
            json!([{ "property": { "key": "vip", "op": "eq", "value": 1 } }]),
        ),
        (
            "eq string",
            json!([{ "property": { "key": "vip", "op": "eq", "value": "yes" } }]),
        ),
        (
            "segment",
            json!([{ "segment": { "segment_id": common_domain::ids::SegmentId::new().as_base62() } }]),
        ),
        (
            "unknown property",
            json!([{ "property": { "key": "nope", "op": "set" } }]),
        ),
        (
            "op/type mismatch",
            json!([{ "property": { "key": "seats", "op": "in", "values": ["1"] } }]),
        ),
        (
            "date property",
            json!([{ "property": { "key": "signed_at", "op": "set" } }]),
        ),
        (
            "option outside config",
            json!([{ "property": { "key": "tier", "op": "in", "values": ["gold"] } }]),
        ),
        (
            "bad plan_type",
            json!([{ "attribute": { "key": "plan_type", "values": ["GOLD"] } }]),
        ),
        (
            "bad currency",
            json!([{ "attribute": { "key": "currency", "values": ["XXX"] } }]),
        ),
        (
            "bad country",
            json!([{ "attribute": { "key": "country", "values": ["ZZ"] } }]),
        ),
        (
            "empty values",
            json!([{ "attribute": { "key": "country", "values": [] } }]),
        ),
        ("empty group", json!([{ "or": { "children": [] } }])),
        (
            "unknown plan",
            json!([{ "plan": { "plan_ids": [common_domain::ids::PlanId::new().as_base62()] } }]),
        ),
        (
            "unknown customer",
            json!([{ "customer": { "customer_ids": [CustomerId::new().as_base62()] } }]),
        ),
        (
            "amount other currency literal",
            json!([{ "amount": { "op": "gte", "threshold": 1, "currency": "XXX" } }]),
        ),
    ];
    for (label, predicates) in cases {
        let result = env
            .store()
            .preview_audience(
                TENANT_ID,
                INVOICING_ENTITY_ID,
                &audience(predicates),
                None,
                10,
            )
            .await;
        assert_invalid_argument(result, label);
    }

    // An empty audience is not an audience, and unknown leaves never deserialize.
    assert_invalid_argument(
        env.store()
            .preview_audience(
                TENANT_ID,
                INVOICING_ENTITY_ID,
                &audience(json!([])),
                None,
                10,
            )
            .await,
        "empty audience",
    );
    assert!(
        meteroid_store::services::payment_reminders::audience::parse_audience(
            r#"{"predicates":[{"tenant":{"ids":[]}}]}"#
        )
        .is_err()
    );

    // Customers of another entity are rejected, the entity's own are fine.
    let other_entity = create_second_entity(&env).await;
    let store = env.store();
    let moved = store
        .patch_customer(
            ACTOR,
            TENANT_ID,
            CustomerPatch {
                invoicing_entity_id: Some(other_entity),
                ..customer_patch(CUST_COMODO_ID)
            },
        )
        .await
        .expect("move customer")
        .expect("customer exists");
    assert_eq!(moved.invoicing_entity_id, other_entity);
    assert_invalid_argument(
        store
            .preview_audience(
                TENANT_ID,
                INVOICING_ENTITY_ID,
                &audience(
                    json!([{ "customer": { "customer_ids": [CUST_COMODO_ID.as_base62()] } }]),
                ),
                None,
                10,
            )
            .await,
        "customer of another entity",
    );
    store
        .preview_audience(
            TENANT_ID,
            INVOICING_ENTITY_ID,
            &audience(json!([
                { "customer": { "customer_ids": [CUST_UBER_ID.as_base62()] } },
                { "property": { "key": "tier", "op": "in", "values": ["enterprise"] } },
                { "property": { "key": "seats", "op": "gte", "value": 10 } },
                { "property": { "key": "seats", "op": "eq", "value": 10 } },
                { "property": { "key": "vip", "op": "eq", "value": true } },
                { "plan": { "plan_ids": [PLAN_LEETCODE_ID.as_base62()] } }
            ])),
            None,
            10,
        )
        .await
        .expect("valid audience");
}

#[rstest]
#[tokio::test]
async fn audience_semantics_on_fixtures(#[future] test_env: TestEnv) {
    let env = test_env.await;
    define_property(
        &env,
        "tier",
        CustomPropertyType::SingleSelect,
        &["enterprise", "smb"],
    )
    .await;
    let acme = insert_customer(&env, "Acme", Some("FR"), json!({ "tier": "enterprise" })).await;
    let globex = insert_customer(&env, "Globex", None, json!({ "tier": "smb" })).await;

    let leetcode_sub: SubscriptionId = subscription()
        .customer(acme)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .create(env.services())
        .await;
    let free_sub: SubscriptionId = subscription()
        .customer(globex)
        .plan_version(PLAN_VERSION_FREE_ID)
        .create(env.services())
        .await;

    let one_off = env.finalized_invoice(FinalizedInvoice::one_off(acme)).await;
    let sub_invoice = env
        .finalized_invoice(FinalizedInvoice::subscription(
            acme,
            leetcode_sub,
            PLAN_VERSION_1_LEETCODE_ID,
        ))
        .await;
    let usd_invoice = env
        .finalized_invoice(FinalizedInvoice::one_off(globex).amount(10_000, "USD"))
        .await;
    let parent = env
        .finalized_invoice(FinalizedInvoice::one_off(globex))
        .await;
    let child_free = env
        .finalized_invoice(
            FinalizedInvoice::subscription(globex, free_sub, PLAN_VERSION_FREE_ID)
                .consolidated_into(parent),
        )
        .await;
    let ids = vec![one_off, sub_invoice, usd_invoice, parent, child_free];
    let set = |ids: &[InvoiceId]| ids.iter().copied().collect::<HashSet<_>>();

    let leetcode = audience(json!([{ "plan": { "plan_ids": [PLAN_LEETCODE_ID.as_base62()] } }]));
    let not_leetcode = audience(
        json!([{ "not": { "child": { "plan": { "plan_ids": [PLAN_LEETCODE_ID.as_base62()] } } } }]),
    );
    assert_eq!(env.matching(&leetcode, &ids).await, set(&[sub_invoice]));
    assert_eq!(
        env.matching(&not_leetcode, &ids).await,
        set(&[one_off, usd_invoice, parent, child_free]),
        "no subscription => plan leaves are FALSE, so not(plan) is TRUE"
    );

    let free_plan = audience(json!([{ "plan": { "plan_ids": [PLAN_FREE_ID.as_base62()] } }]));
    assert_eq!(
        env.matching(&free_plan, &ids).await,
        set(&[parent, child_free]),
        "a consolidated parent matches when ANY member matches"
    );
    let kinds = |values: serde_json::Value| {
        audience(json!([{ "attribute": { "key": "invoice_kind", "values": values } }]))
    };
    assert_eq!(
        env.matching(&kinds(json!(["consolidated"])), &ids).await,
        set(&[parent])
    );
    assert_eq!(
        env.matching(&kinds(json!(["subscription"])), &ids).await,
        set(&[sub_invoice, child_free])
    );
    assert_eq!(
        env.matching(&kinds(json!(["one_off"])), &ids).await,
        set(&[one_off, usd_invoice])
    );
    let standard =
        audience(json!([{ "attribute": { "key": "plan_type", "values": ["STANDARD"] } }]));
    assert_eq!(env.matching(&standard, &ids).await, set(&[sub_invoice]));
    let free_type = audience(json!([{ "attribute": { "key": "plan_type", "values": ["FREE"] } }]));
    assert_eq!(
        env.matching(&free_type, &ids).await,
        set(&[parent, child_free])
    );

    let amount = |currency: &str| {
        audience(json!([{ "amount": { "op": "gte", "threshold": 5000, "currency": currency } }]))
    };
    assert_eq!(
        env.matching(&amount("EUR"), &ids).await,
        set(&[one_off, sub_invoice, parent, child_free])
    );
    assert_eq!(
        env.matching(&amount("USD"), &ids).await,
        set(&[usd_invoice]),
        "an amount in another currency is FALSE"
    );

    let france = audience(json!([{ "attribute": { "key": "country", "values": ["FR"] } }]));
    assert_eq!(
        env.matching(&france, &ids).await,
        set(&[one_off, sub_invoice])
    );
    let not_france = audience(
        json!([{ "not": { "child": { "attribute": { "key": "country", "values": ["FR"] } } } }]),
    );
    assert_eq!(
        env.matching(&not_france, &ids).await,
        set(&[usd_invoice, parent, child_free]),
        "a missing country is FALSE for the positive leaf"
    );

    let enterprise =
        audience(json!([{ "property": { "key": "tier", "op": "in", "values": ["enterprise"] } }]));
    assert_eq!(
        env.matching(&enterprise, &ids).await,
        set(&[one_off, sub_invoice])
    );
    let no_tier = audience(json!([{ "property": { "key": "tier", "op": "not_set" } }]));
    assert!(env.matching(&no_tier, &ids).await.is_empty());

    // Live data: the country moves with the customer's billing address.
    env.store()
        .patch_customer(
            ACTOR,
            TENANT_ID,
            CustomerPatch {
                billing_address: Some(Address {
                    line1: None,
                    line2: None,
                    city: None,
                    country: Some(CountryCode {
                        code: "FR".to_string(),
                        name: "France".to_string(),
                    }),
                    state: None,
                    zip_code: None,
                }),
                ..customer_patch(globex)
            },
        )
        .await
        .expect("patch customer");
    assert_eq!(env.matching(&france, &ids).await, set(&ids));

    // Preview: counts over the entity's open invoices, earlier sequences claim first.
    let acme_only = audience(json!([{ "customer": { "customer_ids": [acme.as_base62()] } }]));
    let earlier = env
        .create_sequence_with_audience(INVOICING_ENTITY_ID, "Acme first", acme_only, &[0], true)
        .await;
    let preview = env
        .store()
        .preview_audience(TENANT_ID, INVOICING_ENTITY_ID, &standard, None, 10)
        .await
        .expect("preview");
    assert!(preview.invoice_count >= 1, "{preview:?}");
    assert_eq!(preview.customer_count, 1, "{preview:?}");
    assert_eq!(
        preview.claimed_by_earlier, preview.invoice_count,
        "every STANDARD invoice is Acme's, claimed by the earlier sequence: {preview:?}"
    );
    assert!(preview.sample.iter().any(|r| r.invoice_id == sub_invoice));
    let free_preview = env
        .store()
        .preview_audience(TENANT_ID, INVOICING_ENTITY_ID, &free_type, None, 10)
        .await
        .expect("preview free");
    assert!(free_preview.sample.iter().any(|r| r.invoice_id == parent));
    assert!(
        free_preview
            .sample
            .iter()
            .all(|r| r.invoice_id != child_free),
        "consolidated children are never candidates"
    );
    let preview_for_earlier = env
        .store()
        .preview_audience(
            TENANT_ID,
            INVOICING_ENTITY_ID,
            &standard,
            Some(earlier.id),
            10,
        )
        .await
        .expect("preview positioned first");
    assert_eq!(preview_for_earlier.claimed_by_earlier, 0);
}

#[rstest]
#[tokio::test]
async fn payment_method_classification_matches_rust_resolver(#[future] test_env: TestEnv) {
    let env = test_env.await;
    // One mock connector routed for both card and direct debit; Uber holds a card and a SEPA
    // mandate on it, Comodo holds nothing.
    env.seed_card_and_direct_debit_same_provider().await;
    env.seed_customer_payment_methods().await;
    env.seed_uber_sepa_payment_method().await;

    let cases: Vec<(&str, CustomerId, Option<PaymentMethodsConfig>)> = vec![
        ("card, no config", CUST_UBER_ID, None),
        (
            "card explicitly online",
            CUST_UBER_ID,
            Some(PaymentMethodsConfig::online()),
        ),
        (
            "card disabled => direct debit",
            CUST_UBER_ID,
            Some(PaymentMethodsConfig::online_specific(false, true)),
        ),
        (
            "both rails disabled",
            CUST_UBER_ID,
            Some(PaymentMethodsConfig::online_specific(false, false)),
        ),
        (
            "transfer with account",
            CUST_UBER_ID,
            Some(PaymentMethodsConfig::BankTransfer {
                account_id: Some(TEST_BANK_ACCOUNT_ID),
            }),
        ),
        (
            "transfer without account",
            CUST_UBER_ID,
            Some(PaymentMethodsConfig::BankTransfer { account_id: None }),
        ),
        (
            "external",
            CUST_UBER_ID,
            Some(PaymentMethodsConfig::External),
        ),
        ("no saved method", CUST_COMODO_ID, None),
    ];

    let mut invoices: Vec<(&str, SubscriptionId, InvoiceId)> = Vec::new();
    for (label, customer, config) in cases {
        let mut builder = subscription()
            .customer(customer)
            .plan_version(PLAN_VERSION_1_LEETCODE_ID);
        if let Some(config) = config {
            builder = builder.payment_methods_config(config);
        }
        let sub = builder.create(env.services()).await;
        let invoice = env
            .finalized_invoice(FinalizedInvoice::subscription(
                customer,
                sub,
                PLAN_VERSION_1_LEETCODE_ID,
            ))
            .await;
        invoices.push((label, sub, invoice));
    }

    assert_parity(&env, &invoices).await;

    // A disabled connector collects nothing on either side.
    let mut conn = env.conn().await;
    ConnectorRow::set_status(
        &mut conn,
        MOCK_CONNECTOR_ID,
        TENANT_ID,
        ConnectorStatusEnum::Disabled,
    )
    .await
    .expect("disable connector");
    drop(conn);
    let expected_none: Vec<_> = invoices
        .iter()
        .filter(|(l, _, _)| l.starts_with("card"))
        .collect();
    assert_eq!(expected_none.len(), 3);
    assert_parity(&env, &invoices).await;
    let none =
        audience(json!([{ "attribute": { "key": "payment_method_type", "values": ["NONE"] } }]));
    let ids: Vec<InvoiceId> = invoices.iter().map(|(_, _, id)| *id).collect();
    let none_matches = env.matching(&none, &ids).await;
    for (label, _, invoice) in &expected_none {
        assert!(
            none_matches.contains(invoice),
            "{label} should read NONE once the connector is disabled"
        );
    }
}

/// SQL classification of every invoice == the Rust resolver's answer for its subscription.
async fn assert_parity(env: &TestEnv, invoices: &[(&str, SubscriptionId, InvoiceId)]) {
    let ids: Vec<InvoiceId> = invoices.iter().map(|(_, _, id)| *id).collect();
    let mut sql_class: HashMap<InvoiceId, &str> = HashMap::new();
    for class in ["CARD", "DIRECT_DEBIT", "TRANSFER", "NONE"] {
        let matches = env
            .matching(
                &audience(
                    json!([{ "attribute": { "key": "payment_method_type", "values": [class] } }]),
                ),
                &ids,
            )
            .await;
        for id in matches {
            let previous = sql_class.insert(id, class);
            assert!(
                previous.is_none(),
                "invoice classified twice: {previous:?} and {class}"
            );
        }
    }

    for (label, sub, invoice) in invoices {
        let resolved = env
            .store()
            .resolve_payment_method_for_subscription(TENANT_ID, *sub)
            .await
            .expect("resolve payment method");
        let expected = match resolved {
            ResolvedPaymentMethod::NotConfigured => "NONE",
            ResolvedPaymentMethod::BankTransfer(_) => "TRANSFER",
            ResolvedPaymentMethod::CustomerPaymentMethod(id) => {
                let mut conn = env.conn().await;
                let method = CustomerPaymentMethodRow::get_by_id(&mut conn, &TENANT_ID, &id)
                    .await
                    .expect("payment method");
                match method.payment_method_type {
                    PaymentMethodTypeEnum::Card => "CARD",
                    PaymentMethodTypeEnum::DirectDebitSepa
                    | PaymentMethodTypeEnum::DirectDebitAch
                    | PaymentMethodTypeEnum::DirectDebitBacs => "DIRECT_DEBIT",
                    other => panic!("unexpected method type {other:?}"),
                }
            }
        };
        assert_eq!(
            sql_class.get(invoice).copied(),
            Some(expected),
            "{label}: SQL classification drifted from resolve_payment_method_for_subscription"
        );
    }
}

#[rstest]
#[tokio::test]
async fn archive_nulls_customer_overrides_and_stops_in_flight(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let sequence = env
        .create_sequence(INVOICING_ENTITY_ID, &[0, 7], true)
        .await;
    let other = env.create_sequence(INVOICING_ENTITY_ID, &[0], true).await;
    let now = Utc::now().naive_utc();

    let pref = env
        .store()
        .set_customer_reminder_preference(
            ACTOR,
            TENANT_ID,
            CUST_UBER_ID,
            CustomerReminderPreference {
                opt_out: false,
                sequence_id: Some(sequence.id),
            },
            now,
        )
        .await
        .expect("set preference");
    assert_eq!(pref.preference.sequence_id, Some(sequence.id));
    assert!(pref.available_sequences.iter().any(|s| s.id == sequence.id));
    assert!(pref.available_sequences.iter().all(|s| !s.archived));

    let in_flight = env
        .finalized_invoice(FinalizedInvoice::one_off(CUST_UBER_ID))
        .await;
    let elsewhere = env
        .finalized_invoice(FinalizedInvoice::one_off(CUST_SPOTIFY_ID))
        .await;
    env.enroll_invoice(in_flight, sequence.id).await;
    env.enroll_invoice(elsewhere, other.id).await;
    let listed = env.list_sequences(INVOICING_ENTITY_ID).await;
    let counted = listed
        .iter()
        .find(|s| s.sequence.id == sequence.id)
        .unwrap();
    assert_eq!(counted.in_flight_invoices, 1);

    let stopped = env
        .store()
        .archive_reminder_sequence(ACTOR, TENANT_ID, sequence.id, now)
        .await
        .expect("archive");
    assert_eq!(stopped, 1);

    let row = env.reminder_row(in_flight).await.expect("row");
    assert!(row.stopped_at.is_some());
    assert_eq!(
        row.stop_reason,
        Some(ReminderStopReasonEnum::SequenceArchived)
    );
    assert!(row.next_reminder_at.is_none());
    let untouched = env.reminder_row(elsewhere).await.expect("row");
    assert!(untouched.stopped_at.is_none() && untouched.next_reminder_at.is_some());

    let customer = env.get_customer(CUST_UBER_ID).await;
    assert_eq!(customer.payment_reminder_sequence_id, None);
    let customer_rows = activities(&env, "customer", CUST_UBER_ID.as_uuid()).await;
    assert!(
        customer_rows
            .iter()
            .any(|r| r.activity_type == "customer.updated"
                && changed_fields(r).contains(&"payment_reminder_sequence_id".to_string())),
        "{customer_rows:?}"
    );
    let sequence_rows = activities(&env, "payment_reminder_sequence", sequence.id.as_uuid()).await;
    assert!(
        sequence_rows
            .iter()
            .any(|r| r.activity_type == "reminder_sequence.archived")
    );

    // Archived sequences leave the live listing, reappear on request, refuse edits and overrides.
    assert!(
        env.list_sequences(INVOICING_ENTITY_ID)
            .await
            .iter()
            .all(|s| s.sequence.id != sequence.id)
    );
    let with_archived = env
        .store()
        .list_reminder_sequences(TENANT_ID, INVOICING_ENTITY_ID, true)
        .await
        .unwrap();
    assert!(
        with_archived
            .iter()
            .any(|s| s.sequence.id == sequence.id && s.sequence.archived_at.is_some())
    );
    assert_invalid_argument(
        env.store()
            .set_customer_reminder_preference(
                ACTOR,
                TENANT_ID,
                CUST_UBER_ID,
                CustomerReminderPreference {
                    opt_out: false,
                    sequence_id: Some(sequence.id),
                },
                now,
            )
            .await,
        "archived override",
    );

    // Opting out stops the customer's remaining in-flight invoices too.
    env.enroll_invoice(in_flight, other.id).await;
    env.store()
        .set_customer_reminder_preference(
            ACTOR,
            TENANT_ID,
            CUST_UBER_ID,
            CustomerReminderPreference {
                opt_out: true,
                sequence_id: None,
            },
            now,
        )
        .await
        .expect("opt out");
    let row = env.reminder_row(in_flight).await.expect("row");
    assert_eq!(
        row.stop_reason,
        Some(ReminderStopReasonEnum::CustomerOptOut)
    );
    let untouched = env.reminder_row(elsewhere).await.expect("row");
    assert!(untouched.stopped_at.is_none());
}

#[rstest]
#[tokio::test]
async fn default_sequence_cannot_be_archived(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let default = env
        .list_sequences(INVOICING_ENTITY_ID)
        .await
        .into_iter()
        .find(|s| s.sequence.is_default())
        .expect("default seeded");
    assert_invalid_argument(
        env.store()
            .archive_reminder_sequence(
                ACTOR,
                TENANT_ID,
                default.sequence.id,
                Utc::now().naive_utc(),
            )
            .await,
        "archive default",
    );
    let still = env.list_sequences(INVOICING_ENTITY_ID).await;
    assert!(
        still
            .iter()
            .any(|s| s.sequence.id == default.sequence.id && s.sequence.archived_at.is_none())
    );

    // The default catches everyone already; it is not offered as an override either.
    assert_invalid_argument(
        env.store()
            .set_customer_reminder_preference(
                ACTOR,
                TENANT_ID,
                CUST_SPOTIFY_ID,
                CustomerReminderPreference {
                    opt_out: false,
                    sequence_id: Some(default.sequence.id),
                },
                Utc::now().naive_utc(),
            )
            .await,
        "default as override",
    );
}

#[rstest]
#[tokio::test]
async fn invoice_kind_subscription_covers_adjustment_and_usage_threshold(
    #[future] test_env: TestEnv,
) {
    let env = test_env.await;
    let acme = insert_customer(&env, "Acme", Some("FR"), json!({})).await;
    let sub: SubscriptionId = subscription()
        .customer(acme)
        .plan_version(PLAN_VERSION_1_LEETCODE_ID)
        .create(env.services())
        .await;
    let fixture = || FinalizedInvoice::subscription(acme, sub, PLAN_VERSION_1_LEETCODE_ID);
    let recurring = env.finalized_invoice(fixture()).await;
    let adjustment = env
        .finalized_invoice(fixture().invoice_type(InvoiceType::Adjustment))
        .await;
    let usage = env
        .finalized_invoice(fixture().invoice_type(InvoiceType::UsageThreshold))
        .await;
    let one_off = env.finalized_invoice(FinalizedInvoice::one_off(acme)).await;
    let ids = vec![recurring, adjustment, usage, one_off];

    let kind = |value: &str| {
        audience(json!([{ "attribute": { "key": "invoice_kind", "values": [value] } }]))
    };
    assert_eq!(
        env.matching(&kind("subscription"), &ids).await,
        HashSet::from([recurring, adjustment, usage])
    );
    assert_eq!(
        env.matching(&kind("one_off"), &ids).await,
        HashSet::from([one_off])
    );
    let leetcode = audience(json!([{ "plan": { "plan_ids": [PLAN_LEETCODE_ID.as_base62()] } }]));
    assert_eq!(
        env.matching(&leetcode, &ids).await,
        HashSet::from([recurring, adjustment, usage])
    );
}

#[rstest]
#[tokio::test]
async fn numeric_property_eq_survives_data_drift(#[future] test_env: TestEnv) {
    let env = test_env.await;
    define_property(&env, "seats", CustomPropertyType::Number, &[]).await;
    let ten = insert_customer(&env, "Ten", None, json!({ "seats": 10 })).await;
    let three = insert_customer(&env, "Three", None, json!({ "seats": 3 })).await;
    let drifted = insert_customer(&env, "Drifted", None, json!({ "seats": 10 })).await;
    let unset = insert_customer(&env, "Unset", None, json!({})).await;
    {
        use diesel_async::RunQueryDsl;
        let mut conn = env.conn().await;
        diesel::sql_query(
            "UPDATE customer SET custom_properties = '{\"seats\": \"many\"}'::jsonb WHERE id = $1",
        )
        .bind::<diesel::sql_types::Uuid, _>(drifted.as_uuid())
        .execute(&mut conn)
        .await
        .expect("drift the property");
    }
    let mut invoices = HashMap::new();
    for customer in [ten, three, drifted, unset] {
        let id = env
            .finalized_invoice(FinalizedInvoice::one_off(customer))
            .await;
        invoices.insert(customer, id);
    }
    let ids: Vec<InvoiceId> = invoices.values().copied().collect();
    let seats = |op: &str, value: serde_json::Value| {
        audience(json!([{ "property": { "key": "seats", "op": op, "value": value } }]))
    };
    let not_seats = |op: &str, value: serde_json::Value| {
        audience(
            json!([{ "not": { "child": { "property": { "key": "seats", "op": op, "value": value } } } }]),
        )
    };

    assert_eq!(
        env.matching(&seats("eq", json!(10)), &ids).await,
        HashSet::from([invoices[&ten]])
    );
    assert_eq!(
        env.matching(&seats("eq", json!(10.0)), &ids).await,
        HashSet::from([invoices[&ten]])
    );
    assert_eq!(
        env.matching(&seats("lt", json!(5)), &ids).await,
        HashSet::from([invoices[&three]])
    );
    assert_eq!(
        env.matching(&not_seats("gte", json!(5)), &ids).await,
        HashSet::from([invoices[&three], invoices[&drifted], invoices[&unset]]),
        "a non-numeric or missing value compares as FALSE, so not() matches it"
    );
}

#[rstest]
#[tokio::test]
async fn reorder_refuses_another_tenants_entity_without_seeding(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let other = env
        .store()
        .insert_tenant(
            TenantNew {
                name: "Other tenant".to_string(),
                environment: TenantEnvironmentEnum::Development,
                email_delivery_mode: Some(meteroid_store::domain::enums::EmailDeliveryMode::Off),
                invoicing_entity: None,
            },
            ORGANIZATION_ID,
        )
        .await
        .expect("insert tenant");
    let foreign_entity = env
        .store()
        .get_invoicing_entity(other.id, None)
        .await
        .expect("other tenant default entity")
        .id;
    // An entity from before the feature: nothing seeded yet.
    {
        use diesel_async::RunQueryDsl;
        let mut conn = env.conn().await;
        diesel::sql_query("DELETE FROM payment_reminder_sequence WHERE invoicing_entity_id = $1")
            .bind::<diesel::sql_types::Uuid, _>(foreign_entity.as_uuid())
            .execute(&mut conn)
            .await
            .expect("unseed");
    }

    let result = env
        .store()
        .reorder_reminder_sequences(ACTOR, TENANT_ID, foreign_entity, vec![])
        .await;
    assert!(
        matches!(
            result.as_ref().map_err(|e| e.current_context()),
            Err(StoreError::ValueNotFound(_))
        ),
        "{result:?}"
    );
    let listed = env
        .store()
        .list_reminder_sequences(TENANT_ID, foreign_entity, true)
        .await;
    assert!(
        matches!(
            listed.as_ref().map_err(|e| e.current_context()),
            Err(StoreError::ValueNotFound(_))
        ),
        "{listed:?}"
    );

    let mut conn = env.conn().await;
    for tenant in [TENANT_ID, other.id] {
        let rows =
            PaymentReminderSequenceRow::list_by_entity(&mut conn, tenant, foreign_entity, true)
                .await
                .unwrap();
        assert!(rows.is_empty(), "{tenant:?} wrote {rows:?}");
    }
    // The seed itself is tenant-safe, whoever calls it.
    let seeded =
        diesel_models::payment_reminders::PaymentReminderSequenceRowNew::insert_default_if_absent(
            &mut conn,
            common_domain::ids::PaymentReminderSequenceId::new(),
            TENANT_ID,
            foreign_entity,
            DEFAULT_SEQUENCE_NAME,
        )
        .await
        .unwrap();
    assert_eq!(seeded, None);
    let rows =
        PaymentReminderSequenceRow::list_by_entity(&mut conn, other.id, foreign_entity, true)
            .await
            .unwrap();
    assert!(rows.is_empty(), "{rows:?}");

    // The owner still gets its default lazily.
    let owned = env
        .store()
        .list_reminder_sequences(other.id, foreign_entity, false)
        .await
        .expect("owner lists");
    assert_eq!(owned.len(), 1);
    assert!(owned[0].sequence.is_default());
    assert_eq!(owned[0].sequence.steps.len(), 4);
}

#[rstest]
#[tokio::test]
async fn concurrent_first_listings_seed_one_complete_default(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let store = env.store();
    let list = || store.list_reminder_sequences(TENANT_ID, INVOICING_ENTITY_ID, false);
    let (a, b, c) = tokio::join!(list(), list(), list());
    for listed in [a.expect("a"), b.expect("b"), c.expect("c")] {
        assert_eq!(listed.len(), 1);
        assert!(listed[0].sequence.is_default());
        assert_eq!(listed[0].sequence.steps.len(), 4);
    }
    let mut conn = env.conn().await;
    let rows =
        PaymentReminderSequenceRow::list_by_entity(&mut conn, TENANT_ID, INVOICING_ENTITY_ID, true)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
}

#[rstest]
#[tokio::test]
async fn customer_entity_change_clears_override(#[future] test_env: TestEnv) {
    let env = test_env.await;
    let sequence = env.create_sequence(INVOICING_ENTITY_ID, &[0], true).await;
    let now = Utc::now().naive_utc();
    env.store()
        .set_customer_reminder_preference(
            ACTOR,
            TENANT_ID,
            CUST_SPOTIFY_ID,
            CustomerReminderPreference {
                opt_out: false,
                sequence_id: Some(sequence.id),
            },
            now,
        )
        .await
        .expect("set preference");
    let before = activities(&env, "customer", CUST_SPOTIFY_ID.as_uuid())
        .await
        .len();

    // Editing an unrelated field keeps the override.
    env.store()
        .patch_customer(
            ACTOR,
            TENANT_ID,
            CustomerPatch {
                name: Some("Spotify AB".to_string()),
                ..customer_patch(CUST_SPOTIFY_ID)
            },
        )
        .await
        .expect("patch name");
    assert_eq!(
        env.get_customer(CUST_SPOTIFY_ID)
            .await
            .payment_reminder_sequence_id,
        Some(sequence.id)
    );

    let other_entity = create_second_entity(&env).await;
    let moved = env
        .store()
        .patch_customer(
            ACTOR,
            TENANT_ID,
            CustomerPatch {
                invoicing_entity_id: Some(other_entity),
                ..customer_patch(CUST_SPOTIFY_ID)
            },
        )
        .await
        .expect("move customer")
        .expect("customer exists");
    assert_eq!(moved.invoicing_entity_id, other_entity);
    assert_eq!(moved.payment_reminder_sequence_id, None);
    assert_eq!(
        env.get_customer(CUST_SPOTIFY_ID)
            .await
            .payment_reminder_sequence_id,
        None
    );

    let rows = activities(&env, "customer", CUST_SPOTIFY_ID.as_uuid()).await;
    assert!(rows.len() > before);
    assert!(
        rows.iter().any(|r| r.activity_type == "customer.updated"
            && changed_fields(r) == vec!["payment_reminder_sequence_id".to_string()]),
        "{rows:?}"
    );

    // The override can only point at a sequence of the customer's (new) entity.
    assert_invalid_argument(
        env.store()
            .set_customer_reminder_preference(
                ACTOR,
                TENANT_ID,
                CUST_SPOTIFY_ID,
                CustomerReminderPreference {
                    opt_out: false,
                    sequence_id: Some(sequence.id),
                },
                now,
            )
            .await,
        "override from another entity",
    );
    let view = env
        .store()
        .get_customer_reminder_preference(TENANT_ID, CUST_SPOTIFY_ID)
        .await
        .expect("get preference");
    assert!(view.available_sequences.is_empty());
}
