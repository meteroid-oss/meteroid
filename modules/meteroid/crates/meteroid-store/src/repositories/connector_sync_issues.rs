use crate::domain::connector_sync_issues::{ConnectorSyncIssue, SyncResource};
use crate::errors::StoreError;
use crate::repositories::{CustomersInterface, InvoiceInterface};
use crate::{Store, StoreResult};
use common_domain::ids::{ConnectorId, TenantId};
use diesel_models::PgConn;
use diesel_models::connector_sync_issues::ConnectorSyncIssueRow;
use diesel_models::connectors::ConnectorRow;
use diesel_models::query::advisory_lock;
use error_stack::Report;

const MAX_LISTED_ISSUES: i64 = 500;

#[async_trait::async_trait]
pub trait ConnectorSyncIssuesInterface {
    async fn record_sync_issue(
        &self,
        tenant_id: TenantId,
        connector_id: ConnectorId,
        resource: SyncResource,
        message: &str,
    ) -> StoreResult<()>;

    async fn clear_sync_issue(
        &self,
        tenant_id: TenantId,
        connector_id: ConnectorId,
        resource: SyncResource,
    ) -> StoreResult<()>;

    async fn list_sync_issues(
        &self,
        tenant_id: TenantId,
        connector_id: ConnectorId,
    ) -> StoreResult<Vec<ConnectorSyncIssue>>;

    /// Sends every listed resource to Pennylane again (one issue per resource); issues clear as
    /// they sync.
    async fn retry_pennylane_sync_issues(
        &self,
        tenant_id: TenantId,
        connector_id: ConnectorId,
    ) -> StoreResult<usize>;

    /// Held while a resource is pushed, so two workers never create it twice. `None` while
    /// another worker holds it.
    async fn try_lock_sync_resource(
        &self,
        resource: SyncResource,
    ) -> StoreResult<Option<SyncResourceLock>>;

    /// The integration refused the connector's credentials; `None` once it accepts them again.
    async fn flag_connector_credentials(
        &self,
        tenant_id: TenantId,
        connector_id: ConnectorId,
        error: Option<String>,
    ) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl ConnectorSyncIssuesInterface for Store {
    async fn record_sync_issue(
        &self,
        tenant_id: TenantId,
        connector_id: ConnectorId,
        resource: SyncResource,
        message: &str,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        let (resource_type, resource_id) = resource.parts();
        ConnectorSyncIssueRow::record(
            &mut conn,
            tenant_id,
            connector_id,
            resource_type,
            resource_id,
            message,
            common_clock::now().naive_utc(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
    }

    async fn clear_sync_issue(
        &self,
        tenant_id: TenantId,
        connector_id: ConnectorId,
        resource: SyncResource,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        let (resource_type, resource_id) = resource.parts();
        ConnectorSyncIssueRow::clear(
            &mut conn,
            tenant_id,
            connector_id,
            resource_type,
            resource_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
    }

    async fn list_sync_issues(
        &self,
        tenant_id: TenantId,
        connector_id: ConnectorId,
    ) -> StoreResult<Vec<ConnectorSyncIssue>> {
        let mut conn = self.get_conn().await?;
        list(&mut conn, tenant_id, connector_id, Some(MAX_LISTED_ISSUES)).await
    }

    async fn retry_pennylane_sync_issues(
        &self,
        tenant_id: TenantId,
        connector_id: ConnectorId,
    ) -> StoreResult<usize> {
        let mut conn = self.get_conn().await?;
        let issues = list(&mut conn, tenant_id, connector_id, None).await?;
        drop(conn);
        let count = issues.len();

        let (mut customers, mut invoices, mut credit_notes) = (vec![], vec![], vec![]);
        for issue in issues {
            match issue.resource {
                SyncResource::Customer(id) => customers.push(id.into()),
                SyncResource::Invoice(id) => invoices.push(id),
                SyncResource::CreditNote(id) => credit_notes.push(id),
            }
        }
        if !customers.is_empty() {
            self.sync_customers_to_pennylane(customers, tenant_id)
                .await?;
        }
        if !invoices.is_empty() {
            self.sync_invoices_to_pennylane(invoices, tenant_id).await?;
        }
        if !credit_notes.is_empty() {
            self.sync_credit_notes_to_pennylane(credit_notes, tenant_id)
                .await?;
        }
        Ok(count)
    }

    async fn try_lock_sync_resource(
        &self,
        resource: SyncResource,
    ) -> StoreResult<Option<SyncResourceLock>> {
        let mut conn = self.get_conn().await?;
        let key = resource_lock_key(resource);
        let acquired = advisory_lock::try_advisory_lock(&mut conn, key)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(acquired.then_some(SyncResourceLock { conn, key }))
    }

    async fn flag_connector_credentials(
        &self,
        tenant_id: TenantId,
        connector_id: ConnectorId,
        error: Option<String>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;
        ConnectorRow::set_credentials_error(&mut conn, connector_id, tenant_id, error)
            .await
            .map(|_| ())
            .map_err(Into::<Report<StoreError>>::into)
    }
}

async fn list(
    conn: &mut PgConn,
    tenant_id: TenantId,
    connector_id: ConnectorId,
    limit: Option<i64>,
) -> StoreResult<Vec<ConnectorSyncIssue>> {
    let rows = ConnectorSyncIssueRow::list(conn, tenant_id, connector_id, limit)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    Ok(rows
        .into_iter()
        .filter_map(ConnectorSyncIssue::from_row)
        .collect())
}

/// A session advisory lock on its own connection, released explicitly: pooled connections
/// outlive their checkout.
pub struct SyncResourceLock {
    conn: PgConn,
    key: i64,
}

impl SyncResourceLock {
    pub async fn release(mut self) {
        if let Err(err) = advisory_lock::advisory_unlock(&mut self.conn, self.key).await {
            log::warn!("Could not release sync lock {}: {err:?}", self.key);
        }
    }
}

/// Keys may collide across resources: a collision only delays one of them.
fn resource_lock_key(resource: SyncResource) -> i64 {
    const NAMESPACE: u64 = 0x5059_4E4C_414E_4531;
    let (_, id) = resource.parts();
    let bits = id.as_u128();
    ((bits as u64) ^ ((bits >> 64) as u64) ^ NAMESPACE) as i64
}
