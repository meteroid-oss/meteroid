//! Sync issues of an accounting integration: recorded per document, counted, cleared once it
//! syncs, and carried over to a reconnected connector.

use crate::data::ids::{CUST_SPOTIFY_ID, TENANT_ID};
use crate::meteroid_it;
use crate::meteroid_it::container::SeedLevel;
use common_domain::ids::{BaseId, ConnectorId, InvoiceId};
use diesel_models::connector_sync_issues::ConnectorSyncIssueRow;
use meteroid_store::Store;
use meteroid_store::domain::connector_sync_issues::SyncResource;
use meteroid_store::domain::connectors::{
    ConnectorConfig, ConnectorNew, PennylanePublicData, PennylaneSensitiveData,
};
use meteroid_store::domain::enums::ConnectorStatusEnum;
use meteroid_store::repositories::connector_sync_issues::ConnectorSyncIssuesInterface;
use meteroid_store::repositories::connectors::ConnectorsInterface;
use secrecy::SecretString;

async fn pennylane_connector(store: &Store, alias: &str) -> ConnectorId {
    let row = ConnectorNew {
        tenant_id: TENANT_ID,
        alias: alias.to_owned(),
        config: ConnectorConfig::Pennylane {
            data: Some(PennylanePublicData {
                external_company_id: "42".into(),
                invoicing_entity_id: None,
            }),
            sensitive: Some(PennylaneSensitiveData {
                refresh_token: SecretString::from("refresh".to_owned()),
                access_token: SecretString::from("access".to_owned()),
                expires_at: None,
            }),
        },
        status: ConnectorStatusEnum::Active,
        account: None,
        webhook_route: None,
    }
    .to_row(&store.settings.crypt_key)
    .unwrap();
    let mut conn = store.get_conn().await.unwrap();
    row.insert(&mut conn).await.unwrap().id
}

#[tokio::test]
async fn test_sync_issues_are_counted_cleared_and_carried_over() {
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::MINIMAL).await;
    let store = &setup.store;

    let first = pennylane_connector(store, "pennylane").await;
    let customer = SyncResource::Customer(CUST_SPOTIFY_ID);
    let invoice = SyncResource::Invoice(InvoiceId::new());

    store
        .record_sync_issue(
            TENANT_ID,
            first,
            customer,
            "Pennylane refused the customer: phone",
        )
        .await
        .unwrap();
    store
        .record_sync_issue(
            TENANT_ID,
            first,
            customer,
            "Pennylane refused the customer: email",
        )
        .await
        .unwrap();
    store
        .record_sync_issue(TENANT_ID, first, invoice, "No VAT code for 12.3% in FR")
        .await
        .unwrap();

    let issues = store.list_sync_issues(TENANT_ID, first).await.unwrap();
    assert_eq!(issues.len(), 2);
    let customer_issue = issues.iter().find(|i| i.resource == customer).unwrap();
    assert_eq!(customer_issue.occurrences, 2);
    assert_eq!(
        customer_issue.message,
        "Pennylane refused the customer: email"
    );

    store
        .clear_sync_issue(TENANT_ID, first, customer)
        .await
        .unwrap();
    let issues = store.list_sync_issues(TENANT_ID, first).await.unwrap();
    assert_eq!(
        issues.iter().map(|i| i.resource).collect::<Vec<_>>(),
        vec![invoice]
    );

    let reconnected = pennylane_connector(store, "pennylane-2").await;
    let mut conn = store.get_conn().await.unwrap();
    ConnectorSyncIssueRow::adopt_from_previous_connectors(&mut conn, TENANT_ID, reconnected)
        .await
        .unwrap();
    assert!(
        store
            .list_sync_issues(TENANT_ID, first)
            .await
            .unwrap()
            .is_empty()
    );
    let carried = store
        .list_sync_issues(TENANT_ID, reconnected)
        .await
        .unwrap();
    assert_eq!(carried.len(), 1);
    assert_eq!(carried[0].resource, invoice);
    assert_eq!(carried[0].message, "No VAT code for 12.3% in FR");

    store
        .flag_connector_credentials(TENANT_ID, reconnected, Some("invalid_token".into()))
        .await
        .unwrap();
    let connector = store
        .get_connector_with_data(reconnected, TENANT_ID)
        .await
        .unwrap();
    assert_eq!(
        connector.credentials_error().map(|e| e.message.as_str()),
        Some("invalid_token")
    );
}

#[tokio::test]
async fn test_a_sync_issue_is_not_recorded_on_another_tenants_connector() {
    let database = meteroid_it::container::create_seeded_test_database(SeedLevel::MINIMAL).await;
    let setup = meteroid_it::container::start_meteroid(database, SeedLevel::MINIMAL).await;
    let store = &setup.store;

    let connector = pennylane_connector(store, "pennylane").await;
    let other_tenant = common_domain::ids::TenantId::new();
    store
        .record_sync_issue(
            other_tenant,
            connector,
            SyncResource::Invoice(InvoiceId::new()),
            "refused",
        )
        .await
        .unwrap();

    assert!(
        store
            .list_sync_issues(TENANT_ID, connector)
            .await
            .unwrap()
            .is_empty()
    );
}
