use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::customproperties::v1::{
    GetCustomPropertyDefinitionRequest, GetCustomPropertyDefinitionResponse,
    ListCustomPropertyDefinitionsRequest, ListCustomPropertyDefinitionsResponse,
    custom_properties_service_server::CustomPropertiesService,
};
use meteroid_store::repositories::CustomPropertyDefinitionsInterface;
use tonic::{Request, Response, Status};

use crate::customproperties::error::CustomPropertiesApiError;

use super::{CustomPropertiesServiceComponents, mapping};

#[tonic::async_trait]
impl CustomPropertiesService for CustomPropertiesServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_custom_property_definitions(
        &self,
        request: Request<ListCustomPropertyDefinitionsRequest>,
    ) -> Result<Response<ListCustomPropertyDefinitionsResponse>, Status> {
        let tenant = request.tenant()?;
        let req = request.into_inner();
        let entity_type = mapping::definitions::entity_type_opt_from_proto(req.entity_type)?;

        let definitions = self
            .store
            .list_custom_property_definitions(tenant, entity_type, req.include_archived)
            .await
            .map_err(Into::<CustomPropertiesApiError>::into)?
            .into_iter()
            .map(mapping::definitions::domain_to_proto)
            .collect();

        Ok(Response::new(ListCustomPropertyDefinitionsResponse {
            definitions,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_custom_property_definition(
        &self,
        request: Request<GetCustomPropertyDefinitionRequest>,
    ) -> Result<Response<GetCustomPropertyDefinitionResponse>, Status> {
        let tenant = request.tenant()?;
        let id = mapping::definitions::parse_id(&request.into_inner().id)?;

        let definition = self
            .store
            .get_custom_property_definition(tenant, id)
            .await
            .map_err(Into::<CustomPropertiesApiError>::into)?;

        Ok(Response::new(GetCustomPropertyDefinitionResponse {
            definition: Some(mapping::definitions::domain_to_proto(definition)),
        }))
    }
}
