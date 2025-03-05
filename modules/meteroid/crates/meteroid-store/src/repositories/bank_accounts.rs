use crate::domain::{BankAccount, BankAccountPatch};
use crate::errors::StoreError;
use crate::store::Store;
use crate::{domain, StoreResult};
use common_domain::ids::{BankAccountId, BaseId, TenantId};
use common_eventbus::Event;
use diesel_models::bank_accounts::{BankAccountRow, BankAccountRowNew, BankAccountRowPatch};
use error_stack::Report;
use uuid::Uuid;

#[async_trait::async_trait]
pub trait BankAccountsInterface {
    async fn list_bank_accounts(&self, tenant_id: TenantId) -> StoreResult<Vec<BankAccount>>;

    async fn get_bank_account_by_id(
        &self,
        id: BankAccountId,
        tenant_id: TenantId,
    ) -> StoreResult<BankAccount>;

    async fn delete_bank_account(&self, id: BankAccountId, tenant_id: TenantId) -> StoreResult<()>;

    async fn insert_bank_account(&self, plan: domain::BankAccountNew) -> StoreResult<BankAccount>;

    async fn patch_bank_account(
        &self,
        plan: BankAccountPatch,
        actor: Uuid,
    ) -> StoreResult<BankAccount>;
}

#[async_trait::async_trait]
impl BankAccountsInterface for Store {
    async fn list_bank_accounts(&self, tenant_id: TenantId) -> StoreResult<Vec<BankAccount>> {
        let mut conn = self.get_conn().await?;

        let bank_accounts = BankAccountRow::list_by_tenant_id(&mut conn, tenant_id.clone())
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(bank_accounts.into_iter().map(Into::into).collect())
    }

    async fn get_bank_account_by_id(
        &self,
        id: BankAccountId,
        tenant_id: TenantId,
    ) -> StoreResult<BankAccount> {
        let mut conn = self.get_conn().await?;

        let bank_account = BankAccountRow::get_by_id(&mut conn, tenant_id, id)
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(bank_account.into())
    }

    async fn delete_bank_account(&self, id: BankAccountId, tenant_id: TenantId) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        BankAccountRow::delete(&mut conn, tenant_id.clone(), id.clone())
            .await
            .map_err(|err| StoreError::DatabaseError(err.error))?;

        Ok(())
    }

    async fn insert_bank_account(
        &self,
        entity: domain::BankAccountNew,
    ) -> StoreResult<domain::BankAccount> {
        let mut conn = self.get_conn().await?;

        let insertable: BankAccountRowNew = entity.into();

        let result: Result<domain::BankAccount, Report<StoreError>> = insertable
            .insert(&mut conn)
            .await
            .map_err(Into::into)
            .map(Into::into);

        if result.is_ok() {
            let _ = self
                .eventbus
                .publish(Event::bank_account_created(
                    insertable.created_by,
                    insertable.id.as_uuid(),
                    insertable.tenant_id.as_uuid(),
                ))
                .await;
        }

        result
    }

    async fn patch_bank_account(
        &self,
        patch: BankAccountPatch,
        actor: Uuid,
    ) -> StoreResult<BankAccount> {
        let mut conn = self.get_conn().await?;

        let patch_row: BankAccountRowPatch = patch.into();

        let result: Result<BankAccount, Report<StoreError>> = patch_row
            .patch(&mut conn)
            .await
            .map_err(Into::into)
            .map(Into::into);

        if result.is_ok() {
            let _ = self
                .eventbus
                .publish(Event::bank_account_edited(
                    actor,
                    patch_row.id.as_uuid(),
                    patch_row.tenant_id.as_uuid(),
                ))
                .await;
        }

        result
    }
}
