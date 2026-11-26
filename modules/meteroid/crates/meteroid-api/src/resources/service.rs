use super::ResourcesServiceComponents;
use super::error::ResourcesApiError;
use super::mapping::{attributes_from_proto, resource_to_proto, type_ref, type_to_proto};
use common_domain::ids::{CustomerId, ResourceId, ResourceTypeId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::resources::v1::resource_types_service_server::ResourceTypesService;
use meteroid_grpc::meteroid::api::resources::v1::resources_service_server::ResourcesService;
use meteroid_grpc::meteroid::api::resources::v1::{
    ArchiveResourceRequest, ArchiveResourceResponse, CreateResourceTypeRequest,
    CreateResourceTypeResponse, DeleteResourceRequest, DeleteResourceResponse, GetResourceRequest,
    GetResourceResponse, GetResourceTypeRequest, GetResourceTypeResponse, ListResourceTypesRequest,
    ListResourceTypesResponse, ListResourcesRequest, ListResourcesResponse,
    UpdateResourceTypeRequest, UpdateResourceTypeResponse, UpsertResourceRequest,
    UpsertResourceResponse,
};
use meteroid_grpc_mapping::utils::PaginationExt;
use meteroid_store::repositories::ResourcesInterface;
use meteroid_store::repositories::resources::{ResourceTypeNew, ResourceTypePatch, ResourceUpsert};
use tonic::{Request, Response, Status};

fn store<T>(result: meteroid_store::StoreResult<T>) -> Result<T, Status> {
    result.map_err(|e| ResourcesApiError::from(e).into())
}

#[tonic::async_trait]
impl ResourceTypesService for ResourcesServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_resource_types(
        &self,
        request: Request<ListResourceTypesRequest>,
    ) -> Result<Response<ListResourceTypesResponse>, Status> {
        let tenant = request.tenant()?;
        let types = store(
            self.store
                .list_resource_types(tenant, request.into_inner().include_archived)
                .await,
        )?;
        Ok(Response::new(ListResourceTypesResponse {
            resource_types: types.into_iter().map(type_to_proto).collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_resource_type(
        &self,
        request: Request<GetResourceTypeRequest>,
    ) -> Result<Response<GetResourceTypeResponse>, Status> {
        let tenant = request.tenant()?;
        let resource_type = type_ref(&request.into_inner().id_or_code);
        let resource_type = store(self.store.get_resource_type(tenant, resource_type).await)?;
        Ok(Response::new(GetResourceTypeResponse {
            resource_type: Some(type_to_proto(resource_type)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_resource_type(
        &self,
        request: Request<CreateResourceTypeRequest>,
    ) -> Result<Response<CreateResourceTypeResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let created = store(
            self.store
                .create_resource_type(ResourceTypeNew {
                    tenant_id,
                    code: req.code,
                    name: req.name,
                    plural_name: req.plural_name,
                    parent_type_id: ResourceTypeId::from_proto_opt(req.parent_type_id)?,
                    keys: req.keys,
                    attributes: attributes_from_proto(req.attributes),
                })
                .await,
        )?;
        Ok(Response::new(CreateResourceTypeResponse {
            resource_type: Some(type_to_proto(created)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_resource_type(
        &self,
        request: Request<UpdateResourceTypeRequest>,
    ) -> Result<Response<UpdateResourceTypeResponse>, Status> {
        let tenant = request.tenant()?;
        let req = request.into_inner();
        let patch = ResourceTypePatch {
            name: req.name,
            plural_name: req.plural_name,
            parent_type_id: ResourceTypeId::from_proto_opt(req.parent_type_id)?,
            keys: req.keys.map(|k| k.keys),
            attributes: req.attributes.map(|a| attributes_from_proto(a.attributes)),
            archived: req.archived,
        };
        let updated = store(
            self.store
                .update_resource_type(tenant, type_ref(&req.id_or_code), patch)
                .await,
        )?;
        Ok(Response::new(UpdateResourceTypeResponse {
            resource_type: Some(type_to_proto(updated)),
        }))
    }
}

#[tonic::async_trait]
impl ResourcesService for ResourcesServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_resources(
        &self,
        request: Request<ListResourcesRequest>,
    ) -> Result<Response<ListResourcesResponse>, Status> {
        let tenant = request.tenant()?;
        let req = request.into_inner();
        let page = store(
            self.store
                .list_resources(
                    tenant,
                    CustomerId::from_proto_opt(req.customer_id)?,
                    req.resource_type.as_deref().map(type_ref),
                    req.include_archived,
                    req.pagination.into_domain(),
                )
                .await,
        )?;
        Ok(Response::new(ListResourcesResponse {
            pagination_meta: req
                .pagination
                .into_response(page.total_pages, page.total_results),
            resources: page.items.into_iter().map(resource_to_proto).collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_resource(
        &self,
        request: Request<GetResourceRequest>,
    ) -> Result<Response<GetResourceResponse>, Status> {
        let tenant = request.tenant()?;
        let id = ResourceId::from_proto(request.into_inner().id)?;
        let resource = store(self.store.get_resource(tenant, id).await)?;
        Ok(Response::new(GetResourceResponse {
            resource: Some(resource_to_proto(resource)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn upsert_resource(
        &self,
        request: Request<UpsertResourceRequest>,
    ) -> Result<Response<UpsertResourceResponse>, Status> {
        let tenant_id = request.tenant()?;
        let req = request.into_inner();
        let resource = store(
            self.store
                .upsert_resource(ResourceUpsert {
                    tenant_id,
                    customer_id: CustomerId::from_proto(req.customer_id)?,
                    resource_type: type_ref(&req.resource_type),
                    key_value: req.key_value,
                    name: req.name,
                    attributes: req.attributes.into_iter().collect(),
                    archived: req.archived,
                })
                .await,
        )?;
        Ok(Response::new(UpsertResourceResponse {
            resource: Some(resource_to_proto(resource)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn archive_resource(
        &self,
        request: Request<ArchiveResourceRequest>,
    ) -> Result<Response<ArchiveResourceResponse>, Status> {
        let tenant = request.tenant()?;
        let id = ResourceId::from_proto(request.into_inner().id)?;
        let resource = store(self.store.set_resource_archived(tenant, id, true).await)?;
        Ok(Response::new(ArchiveResourceResponse {
            resource: Some(resource_to_proto(resource)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn delete_resource(
        &self,
        request: Request<DeleteResourceRequest>,
    ) -> Result<Response<DeleteResourceResponse>, Status> {
        let tenant = request.tenant()?;
        let id = ResourceId::from_proto(request.into_inner().id)?;
        store(self.store.delete_resource(tenant, id).await)?;
        Ok(Response::new(DeleteResourceResponse {}))
    }
}
