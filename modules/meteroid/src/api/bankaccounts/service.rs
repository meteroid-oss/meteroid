use common_domain::ids::{BankAccountId, BaseId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::bankaccounts::v1::{
    CreateBankAccountRequest, CreateBankAccountResponse, DeleteBankAccountRequest,
    DeleteBankAccountResponse, ListBankAccountsRequest, ListBankAccountsResponse,
    UpdateBankAccountRequest, UpdateBankAccountResponse,
    bank_accounts_service_server::BankAccountsService,
};
use tonic::{Request, Response, Status};

use meteroid_store::repositories::bank_accounts::BankAccountsInterface;

use crate::api::bankaccounts::error::BankAccountsApiError;

use super::{BankAccountsServiceComponents, mapping};

#[tonic::async_trait]
impl BankAccountsService for BankAccountsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_bank_accounts(
        &self,
        request: Request<ListBankAccountsRequest>,
    ) -> Result<Response<ListBankAccountsResponse>, Status> {
        let tenant = request.tenant()?;

        let bank_accounts = self
            .store
            .list_bank_accounts(tenant)
            .await
            .map_err(Into::<BankAccountsApiError>::into)?
            .into_iter()
            .map(mapping::bank_accounts::domain_to_proto)
            .collect();

        let response = ListBankAccountsResponse {
            accounts: bank_accounts,
        };

        Ok(Response::new(response))
    }

    #[tracing::instrument(skip_all)]
    async fn create_bank_account(
        &self,
        request: Request<CreateBankAccountRequest>,
    ) -> Result<Response<CreateBankAccountResponse>, Status> {
        let tenant = request.tenant()?;
        let actor = request.actor()?;

        let data = request
            .into_inner()
            .data
            .ok_or_else(|| Status::invalid_argument("Missing data"))?;

        let res = self
            .store
            .insert_bank_account(
                mapping::bank_accounts::proto_to_domain(data, tenant, actor)
                    .map_err(Into::<Status>::into)?,
            )
            .await
            .map_err(Into::<BankAccountsApiError>::into)?;

        Ok(Response::new(CreateBankAccountResponse {
            account: Some(mapping::bank_accounts::domain_to_proto(res)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_bank_account(
        &self,
        request: Request<UpdateBankAccountRequest>,
    ) -> Result<Response<UpdateBankAccountResponse>, Status> {
        let tenant = request.tenant()?;
        let req = request.into_inner();

        let res = self
            .store
            .patch_bank_account(
                mapping::bank_accounts::proto_to_patch_domain(req, tenant)
                    .map_err(Into::<Status>::into)?,
                tenant.as_uuid(),
            )
            .await
            .map_err(Into::<BankAccountsApiError>::into)?;

        Ok(Response::new(UpdateBankAccountResponse {
            account: Some(mapping::bank_accounts::domain_to_proto(res)),
        }))
    }
    #[tracing::instrument(skip_all)]
    async fn delete_bank_account(
        &self,
        request: Request<DeleteBankAccountRequest>,
    ) -> Result<Response<DeleteBankAccountResponse>, Status> {
        let tenant = request.tenant()?;
        let id = BankAccountId::from_proto(request.into_inner().id)?;

        self.store
            .delete_bank_account(id, tenant)
            .await
            .map_err(Into::<BankAccountsApiError>::into)?;

        Ok(Response::new(DeleteBankAccountResponse {}))
    }
}
