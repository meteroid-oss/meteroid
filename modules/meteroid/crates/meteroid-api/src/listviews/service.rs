use crate::listviews::ListViewsServiceComponents;
use crate::listviews::error::ListViewsApiError;
use crate::listviews::mapping::list_views as mapping;
use common_domain::ids::EntityListViewId;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::listviews::v1::list_views_service_server::ListViewsService;
use meteroid_grpc::meteroid::api::listviews::v1::{
    CreateViewRequest, CreateViewResponse, DeleteViewRequest, DeleteViewResponse,
    DuplicateViewRequest, DuplicateViewResponse, ListViewsRequest, ListViewsResponse,
    SetDefaultViewRequest, SetDefaultViewResponse, UpdateViewRequest, UpdateViewResponse,
};
use meteroid_store::domain::entity_list_views::{EntityListViewNew, EntityListViewPatch};
use meteroid_store::repositories::entity_list_views::EntityListViewsInterface;
use tonic::{Request, Response, Status};

// Views are per-user UI state: every handler requires a user actor (`actor_user` rejects API
// keys), and visibility/ownership is enforced by the store.

#[tonic::async_trait]
impl ListViewsService for ListViewsServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn list_views(
        &self,
        request: Request<ListViewsRequest>,
    ) -> Result<Response<ListViewsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_user()?;
        let req = request.into_inner();

        let entity_type = mapping::entity_type_from_proto(req.entity_type)?;

        let res = self
            .store
            .list_views(tenant_id, actor, entity_type)
            .await
            .map_err(Into::<ListViewsApiError>::into)?;

        Ok(Response::new(ListViewsResponse {
            views: res
                .views
                .iter()
                .map(|v| mapping::entity_list_view_to_server(v, actor))
                .collect(),
            default_view_id: res.default_view_id.map(|id| id.as_proto()),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn create_view(
        &self,
        request: Request<CreateViewRequest>,
    ) -> Result<Response<CreateViewResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_user()?;
        let req = request.into_inner();

        let new = EntityListViewNew {
            entity_type: mapping::entity_type_from_proto(req.entity_type)?,
            name: req.name,
            config: mapping::parse_config(&req.config)?,
            shared: req.shared,
        };

        let created = self
            .store
            .create_view(tenant_id, actor, new)
            .await
            .map_err(Into::<ListViewsApiError>::into)?;

        Ok(Response::new(CreateViewResponse {
            view: Some(mapping::entity_list_view_to_server(&created, actor)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn update_view(
        &self,
        request: Request<UpdateViewRequest>,
    ) -> Result<Response<UpdateViewResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_user()?;
        let req = request.into_inner();

        let id = EntityListViewId::from_proto(&req.id)?;
        let patch = EntityListViewPatch {
            name: req.name,
            config: req
                .config
                .as_deref()
                .map(mapping::parse_config)
                .transpose()?,
            shared: req.shared,
        };

        let updated = self
            .store
            .update_view(tenant_id, actor, id, patch)
            .await
            .map_err(Into::<ListViewsApiError>::into)?;

        Ok(Response::new(UpdateViewResponse {
            view: Some(mapping::entity_list_view_to_server(&updated, actor)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn delete_view(
        &self,
        request: Request<DeleteViewRequest>,
    ) -> Result<Response<DeleteViewResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_user()?;
        let id = EntityListViewId::from_proto(&request.get_ref().id)?;

        self.store
            .delete_view(tenant_id, actor, id)
            .await
            .map_err(Into::<ListViewsApiError>::into)?;

        Ok(Response::new(DeleteViewResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn duplicate_view(
        &self,
        request: Request<DuplicateViewRequest>,
    ) -> Result<Response<DuplicateViewResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_user()?;
        let req = request.into_inner();

        let id = EntityListViewId::from_proto(&req.id)?;

        let duplicated = self
            .store
            .duplicate_view(tenant_id, actor, id, req.name)
            .await
            .map_err(Into::<ListViewsApiError>::into)?;

        Ok(Response::new(DuplicateViewResponse {
            view: Some(mapping::entity_list_view_to_server(&duplicated, actor)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn set_default_view(
        &self,
        request: Request<SetDefaultViewRequest>,
    ) -> Result<Response<SetDefaultViewResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_user()?;
        let req = request.into_inner();

        let entity_type = mapping::entity_type_from_proto(req.entity_type)?;
        let view_id = req
            .view_id
            .as_deref()
            .map(EntityListViewId::from_proto)
            .transpose()?;

        let default_view_id = self
            .store
            .set_default_view(tenant_id, actor, entity_type, view_id)
            .await
            .map_err(Into::<ListViewsApiError>::into)?;

        Ok(Response::new(SetDefaultViewResponse {
            default_view_id: default_view_id.map(|id| id.as_proto()),
        }))
    }
}
