use std::sync::Arc;

use chrono::Utc;
use common_domain::ids::{BaseId, BatchJobChunkId, BatchJobId};
use meteroid::workers::batch_jobs::engine::BatchJobProcessor;
use meteroid::workers::batch_jobs::processors::customer_csv::CustomerCsvProcessor;
use meteroid_store::Store;
use meteroid_store::domain::batch_jobs::{BatchJob, BatchJobChunk};
use meteroid_store::domain::custom_property_definitions::{
    CustomPropertyDefinitionNew, PropertyConfig, SelectOption,
};
use meteroid_store::domain::enums::{
    BatchJobChunkStatusEnum, BatchJobStatusEnum, BatchJobTypeEnum, CustomPropertyEntityType,
    CustomPropertyType, CustomerType,
};
use meteroid_store::repositories::CustomersInterface;
use meteroid_testkit::custom_properties::create_custom_property_definition;
use serde_json::json;

use crate::data::ids::{TENANT_ID, USER_ID};
use crate::helpers;
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;

fn job() -> BatchJob {
    let now = Utc::now().naive_utc();
    BatchJob {
        id: BatchJobId::new(),
        tenant_id: TENANT_ID,
        job_type: BatchJobTypeEnum::CustomerCsvImport,
        status: BatchJobStatusEnum::Pending,
        input_source_key: None,
        input_params: Some(json!({ "delimiter": ",", "fail_on_error": false })),
        total_items: None,
        processed_items: 0,
        failed_items: 0,
        file_hash: None,
        locked_at: None,
        created_by: USER_ID.as_uuid(),
        created_at: now,
        updated_at: now,
        completed_at: None,
        error_message: None,
        error_output_key: None,
        input_file_name: None,
        output_key: None,
    }
}

fn chunk(job_id: BatchJobId, item_count: i32) -> BatchJobChunk {
    let now = Utc::now().naive_utc();
    BatchJobChunk {
        id: BatchJobChunkId::new(),
        job_id,
        tenant_id: TENANT_ID,
        chunk_index: 0,
        status: BatchJobChunkStatusEnum::Pending,
        item_offset: 0,
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

async fn define(
    store: &Store,
    key: &str,
    property_type: CustomPropertyType,
    config: PropertyConfig,
) {
    create_custom_property_definition(
        store,
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

async fn properties_of(store: &Store, alias: &str) -> serde_json::Value {
    store
        .find_customer_by_alias(alias.to_string(), TENANT_ID)
        .await
        .unwrap()
        .custom_properties
}

async fn locales_of(store: &Store, alias: &str) -> Vec<String> {
    store
        .find_customer_by_alias(alias.to_string(), TENANT_ID)
        .await
        .unwrap()
        .preferred_locales
}

#[tokio::test]
async fn test_customer_csv_import_custom_properties() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let store = setup.store.clone();
    define(
        &store,
        "tier",
        CustomPropertyType::Text,
        PropertyConfig::default(),
    )
    .await;
    define(
        &store,
        "seats",
        CustomPropertyType::Number,
        PropertyConfig::default(),
    )
    .await;
    define(
        &store,
        "vip",
        CustomPropertyType::Boolean,
        PropertyConfig::default(),
    )
    .await;
    define(
        &store,
        "region",
        CustomPropertyType::SingleSelect,
        PropertyConfig {
            options: Some(vec![
                SelectOption {
                    value: "emea".into(),
                    label: None,
                },
                SelectOption {
                    value: "amer".into(),
                    label: None,
                },
            ]),
            ..PropertyConfig::default()
        },
    )
    .await;

    let processor = CustomerCsvProcessor::new(Arc::new(store.clone()));
    let job = job();

    // --- 1. Import with custom properties, plus a row that provides none.
    let csv = "name,alias,currency,custom_properties.tier,custom_properties.seats,custom_properties.vip,custom_properties.region\n\
        Acme,acme,EUR,enterprise,42,yes,emea\n\
        Globex,globex,EUR,,,,\n";

    let chunks = processor
        .prepare_chunks(&job, Some(csv.as_bytes().to_vec().into()))
        .await
        .unwrap();
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].item_count, 2);

    let result = processor
        .process_chunk(
            &job,
            &chunk(job.id, 2),
            Some(csv.as_bytes().to_vec().into()),
        )
        .await
        .unwrap();
    assert!(result.failures.is_empty(), "{:?}", result.failures);
    assert_eq!(result.processed, 2);

    assert_eq!(
        properties_of(&store, "acme").await,
        json!({ "tier": "enterprise", "seats": 42, "vip": true, "region": "emea" })
    );
    // The file declares the columns, so a row leaving them all empty declares an empty bag.
    assert_eq!(properties_of(&store, "globex").await, json!({}));

    // --- 2. Re-importing with a narrower set of columns replaces the whole bag: the file is
    // the source of truth for custom properties, like a full update.
    let csv = "name,alias,currency,custom_properties.seats\n\
        Acme Inc,acme,EUR,50\n";

    let result = processor
        .process_chunk(
            &job,
            &chunk(job.id, 1),
            Some(csv.as_bytes().to_vec().into()),
        )
        .await
        .unwrap();
    assert!(result.failures.is_empty(), "{:?}", result.failures);

    assert_eq!(properties_of(&store, "acme").await, json!({ "seats": 50 }));

    // --- 3. An empty cell in a declared column clears that property.
    let csv = "name,alias,currency,custom_properties.seats,custom_properties.tier\n\
        Acme Inc,acme,EUR,,gold\n";
    let result = processor
        .process_chunk(
            &job,
            &chunk(job.id, 1),
            Some(csv.as_bytes().to_vec().into()),
        )
        .await
        .unwrap();
    assert!(result.failures.is_empty(), "{:?}", result.failures);
    assert_eq!(
        properties_of(&store, "acme").await,
        json!({ "tier": "gold" })
    );

    // --- 4. A file without any custom-property column declares nothing and leaves the stored
    // bag untouched — existing imports that predate the feature keep working.
    let csv = "name,alias,currency\nAcme Corp,acme,EUR\n";
    let result = processor
        .process_chunk(
            &job,
            &chunk(job.id, 1),
            Some(csv.as_bytes().to_vec().into()),
        )
        .await
        .unwrap();
    assert!(result.failures.is_empty(), "{:?}", result.failures);
    assert_eq!(
        properties_of(&store, "acme").await,
        json!({ "tier": "gold" })
    );

    // --- 5. Bad values fail their own row only.
    let csv = "name,alias,currency,custom_properties.seats,custom_properties.region\n\
        Bad Number,badnum,EUR,many,emea\n\
        Bad Option,badopt,EUR,1,antarctica\n\
        Good,good,EUR,7,amer\n";

    let result = processor
        .process_chunk(
            &job,
            &chunk(job.id, 3),
            Some(csv.as_bytes().to_vec().into()),
        )
        .await
        .unwrap();

    assert_eq!(result.processed, 1);
    assert_eq!(result.failures.len(), 2);
    assert!(
        result.failures.iter().any(|f| f.item_index == 0
            && f.reason.contains("seats")
            && f.reason.contains("number")),
        "{:?}",
        result.failures
    );
    assert!(
        result
            .failures
            .iter()
            .any(|f| f.item_index == 1 && f.reason.contains("antarctica")),
        "{:?}",
        result.failures
    );
    assert_eq!(
        properties_of(&store, "good").await,
        json!({ "seats": 7, "region": "amer" })
    );
    assert!(
        store
            .find_customer_by_alias("badnum".to_string(), TENANT_ID)
            .await
            .is_err()
    );

    // --- 6. An unknown custom-property column fails the job upfront.
    let csv = "name,alias,currency,custom_properties.nope\nNope,nope,EUR,x\n";
    let err = processor
        .prepare_chunks(&job, Some(csv.as_bytes().to_vec().into()))
        .await
        .unwrap_err();
    assert!(err.contains("nope"), "{err}");
    assert!(err.contains("tier"), "{err}");

    setup.token.cancel();
}

#[tokio::test]
async fn test_customer_csv_import_preferred_locales() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let store = setup.store.clone();
    let processor = CustomerCsvProcessor::new(Arc::new(store.clone()));
    let job = job();

    // --- 1. The column sets the override; a blank cell leaves it inheriting (empty).
    let csv = "name,alias,currency,preferred_locales\n\
        Acme,acme,EUR,\"fr-FR,en\"\n\
        Globex,globex,EUR,\n";
    let result = processor
        .process_chunk(
            &job,
            &chunk(job.id, 2),
            Some(csv.as_bytes().to_vec().into()),
        )
        .await
        .unwrap();
    assert!(result.failures.is_empty(), "{:?}", result.failures);
    assert_eq!(locales_of(&store, "acme").await, vec!["fr-FR", "en"]);
    assert!(locales_of(&store, "globex").await.is_empty());

    // --- 2. Re-importing with the column updates the stored override. The deprecated
    // single-value column still works, as a one-entry list.
    let csv = "name,alias,currency,invoicing_language\nAcme,acme,EUR,en-US\n";
    let result = processor
        .process_chunk(
            &job,
            &chunk(job.id, 1),
            Some(csv.as_bytes().to_vec().into()),
        )
        .await
        .unwrap();
    assert!(result.failures.is_empty(), "{:?}", result.failures);
    assert_eq!(locales_of(&store, "acme").await, vec!["en-US"]);

    // --- 3. A file without the column leaves the stored override untouched, so a bulk
    // re-import never wipes a language set through the API/UI.
    let csv = "name,alias,currency\nAcme,acme,EUR\n";
    let result = processor
        .process_chunk(
            &job,
            &chunk(job.id, 1),
            Some(csv.as_bytes().to_vec().into()),
        )
        .await
        .unwrap();
    assert!(result.failures.is_empty(), "{:?}", result.failures);
    assert_eq!(locales_of(&store, "acme").await, vec!["en-US"]);

    // --- 4. A blank cell is treated as "not provided" and also leaves it untouched.
    let csv = "name,alias,currency,preferred_locales\nAcme,acme,EUR,\n";
    let result = processor
        .process_chunk(
            &job,
            &chunk(job.id, 1),
            Some(csv.as_bytes().to_vec().into()),
        )
        .await
        .unwrap();
    assert!(result.failures.is_empty(), "{:?}", result.failures);
    assert_eq!(locales_of(&store, "acme").await, vec!["en-US"]);

    // --- 5. A list we can serve no entry of fails its row rather than being stored and
    // silently ignored; one servable entry alongside it is enough to pass.
    let csv = "name,alias,currency,preferred_locales\n\
        Acme,acme,EUR,\"pl-PL,de-DE\"\n\
        Initech,initech,EUR,pl-PL\n";
    let result = processor
        .process_chunk(
            &job,
            &chunk(job.id, 2),
            Some(csv.as_bytes().to_vec().into()),
        )
        .await
        .unwrap();
    assert_eq!(result.failures.len(), 1, "{:?}", result.failures);
    assert!(
        result.failures.iter().any(|f| f.item_index == 1
            && f.reason.contains("preferred_locales")
            && f.reason.contains("pl-PL")),
        "{:?}",
        result.failures
    );
    assert_eq!(locales_of(&store, "acme").await, vec!["pl-PL", "de-DE"]);

    setup.token.cancel();
}

#[tokio::test]
async fn test_customer_csv_import_reimport_updates_all_fields() {
    helpers::init::logging();
    let postgres_connection_string =
        meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup =
        meteroid_it::container::start_meteroid(postgres_connection_string, SeedLevel::MINIMAL)
            .await;

    let store = setup.store.clone();
    let processor = CustomerCsvProcessor::new(Arc::new(store.clone()));
    let job = job();

    let import = |csv: &'static str| {
        let processor = &processor;
        let job = &job;
        async move {
            let result = processor
                .process_chunk(job, &chunk(job.id, 1), Some(csv.as_bytes().to_vec().into()))
                .await
                .unwrap();
            assert!(result.failures.is_empty(), "{:?}", result.failures);
        }
    };

    import(
        "name,alias,currency,legal_number,buyer_reference,is_tax_exempt,exemption_reason\n\
        Acme,acme,EUR,552100554,LEITWEG-1,true,Non-profit\n",
    )
    .await;
    let created = store
        .find_customer_by_alias("acme".to_string(), TENANT_ID)
        .await
        .unwrap();
    assert_eq!(created.legal_number.as_deref(), Some("552100554"));
    assert_eq!(created.buyer_reference.as_deref(), Some("LEITWEG-1"));
    assert_eq!(created.exemption_reason.as_deref(), Some("Non-profit"));

    import(
        "name,alias,currency,customer_type,first_name,last_name,billing_address.country,legal_number,buyer_reference\n\
        ,acme,EUR,INDIVIDUAL,Ada,Lovelace,FR,732829320,LEITWEG-2\n",
    )
    .await;
    let updated = store
        .find_customer_by_alias("acme".to_string(), TENANT_ID)
        .await
        .unwrap();
    assert_eq!(updated.id, created.id);
    assert_eq!(updated.customer_type, CustomerType::Individual);
    assert_eq!(updated.first_name.as_deref(), Some("Ada"));
    assert_eq!(updated.last_name.as_deref(), Some("Lovelace"));
    assert_eq!(updated.legal_number.as_deref(), Some("732829320"));
    assert_eq!(updated.buyer_reference.as_deref(), Some("LEITWEG-2"));
    assert_eq!(updated.exemption_reason, None);

    setup.token.cancel();
}
