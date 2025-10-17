use super::{TaxesServiceComponents, mapping};
use common_domain::ids::{CustomTaxId, InvoicingEntityId, ProductId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::taxes::v1::taxes_service_server::TaxesService;
use meteroid_grpc::meteroid::api::taxes::v1::{
    self as server, CreateCustomTaxRequest, CreateCustomTaxResponse, DeleteCustomTaxRequest,
    GetProductAccountingRequest, GetProductAccountingResponse, ListCustomTaxesRequest,
    ListCustomTaxesResponse, UpdateCustomTaxRequest, UpdateCustomTaxResponse,
    UpsertProductAccountingRequest, UpsertProductAccountingResponse, ValidateVatNumberRequest,
    ValidateVatNumberResponse,
};
use meteroid_store::repositories::accounting::AccountingInterface;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl TaxesService for TaxesServiceComponents {
    async fn create_custom_tax(
        &self,
        request: Request<CreateCustomTaxRequest>,
    ) -> Result<Response<CreateCustomTaxResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let custom_tax_new = mapping::custom_tax_new_from_server(
            req.custom_tax
                .ok_or_else(|| Status::invalid_argument("custom_tax is required"))?,
        )
        .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let custom_tax = self
            .store
            .insert_custom_tax(tenant_id, custom_tax_new)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(CreateCustomTaxResponse {
            custom_tax: Some(mapping::custom_tax_to_server(custom_tax)),
        }))
    }

    async fn update_custom_tax(
        &self,
        request: Request<UpdateCustomTaxRequest>,
    ) -> Result<Response<UpdateCustomTaxResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let custom_tax = req
            .custom_tax
            .ok_or_else(|| Status::invalid_argument("custom_tax is required"))?;

        let id = CustomTaxId::from_proto(custom_tax.id)?;
        let invoicing_entity_id = InvoicingEntityId::from_proto(custom_tax.invoicing_entity_id)?;

        let custom_tax_domain = meteroid_store::domain::accounting::CustomTax {
            id,
            invoicing_entity_id,
            name: custom_tax.name,
            tax_code: custom_tax.tax_code,
            rules: custom_tax
                .rules
                .into_iter()
                .map(mapping::tax_rule_from_server)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Status::invalid_argument(e.to_string()))?,
        };

        let updated_tax = self
            .store
            .update_custom_tax(tenant_id, custom_tax_domain)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(UpdateCustomTaxResponse {
            custom_tax: Some(mapping::custom_tax_to_server(updated_tax)),
        }))
    }

    async fn delete_custom_tax(
        &self,
        request: Request<DeleteCustomTaxRequest>,
    ) -> Result<Response<()>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let id = CustomTaxId::from_proto(req.id)?;

        self.store
            .delete_custom_tax(tenant_id, id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(()))
    }

    async fn list_custom_taxes(
        &self,
        request: Request<ListCustomTaxesRequest>,
    ) -> Result<Response<ListCustomTaxesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let invoicing_entity_id = InvoicingEntityId::from_proto(req.invoicing_entity_id)?;

        let custom_taxes = self
            .store
            .list_custom_taxes_by_invoicing_entity_id(tenant_id, invoicing_entity_id)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(ListCustomTaxesResponse {
            custom_taxes: custom_taxes
                .into_iter()
                .map(mapping::custom_tax_to_server)
                .collect(),
        }))
    }

    async fn upsert_product_accounting(
        &self,
        request: Request<UpsertProductAccountingRequest>,
    ) -> Result<Response<UpsertProductAccountingResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let product_accounting = mapping::product_accounting_from_server(
            req.product_accounting
                .ok_or_else(|| Status::invalid_argument("product_accounting is required"))?,
        )
        .map_err(|e| Status::invalid_argument(e.to_string()))?;

        let result = self
            .store
            .upsert_product_accounting(tenant_id, product_accounting)
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        Ok(Response::new(UpsertProductAccountingResponse {
            product_accounting: Some(mapping::product_accounting_to_server(result)),
        }))
    }

    async fn get_product_accounting(
        &self,
        request: Request<GetProductAccountingRequest>,
    ) -> Result<Response<GetProductAccountingResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();

        let product_id = ProductId::from_proto(req.product_id)?;
        let invoicing_entity_id = InvoicingEntityId::from_proto(req.invoicing_entity_id)?;

        let mut conn = self
            .store
            .get_conn()
            .await
            .map_err(|e| Status::internal(e.to_string()))?;
        let product_accountings = self
            .store
            .list_product_tax_configuration_by_product_ids_and_invoicing_entity_id_grouped(
                &mut conn,
                tenant_id,
                vec![product_id],
                invoicing_entity_id,
            )
            .await
            .map_err(|e| Status::internal(e.to_string()))?;

        let product_accounting = product_accountings.into_iter().next().map(|pa| {
            mapping::product_accounting_to_server(
                meteroid_store::domain::accounting::ProductAccounting {
                    product_id: pa.product_id,
                    invoicing_entity_id: pa.invoicing_entity_id,
                    product_code: pa.product_code,
                    ledger_account_code: pa.ledger_account_code,
                },
            )
        });

        Ok(Response::new(GetProductAccountingResponse {
            product_accounting,
        }))
    }

    async fn validate_vat_number(
        &self,
        request: Request<ValidateVatNumberRequest>,
    ) -> Result<Response<ValidateVatNumberResponse>, Status> {
        let req = request.into_inner();

        let is_valid = meteroid_tax::validation::validate_vat_number_format(&req.vat_number);

        // TODO: Implement external VIES validation service call
        let status = if is_valid {
            server::validate_vat_number_response::ValidationStatus::Valid
        } else {
            server::validate_vat_number_response::ValidationStatus::Invalid
        };

        Ok(Response::new(ValidateVatNumberResponse {
            is_valid,
            status: status as i32,
            company_name: None,
            company_address: None,
        }))
    }
}
