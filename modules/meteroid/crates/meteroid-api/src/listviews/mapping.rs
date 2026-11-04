pub mod list_views {
    use crate::listviews::error::ListViewsApiError;
    use common_domain::ids::UserId;
    use meteroid_app::conversions::{AsProtoOpt, ProtoConv};
    use meteroid_grpc::meteroid::api::listviews::v1 as server;
    use meteroid_store::domain::entity_list_views as domain;
    use meteroid_store::domain::enums::EntityListType;

    pub fn entity_type_from_proto(v: i32) -> Result<EntityListType, ListViewsApiError> {
        use EntityListType as D;
        use server::EntityListType as P;
        match P::try_from(v).unwrap_or(P::Unspecified) {
            P::Customer => Ok(D::Customer),
            P::Subscription => Ok(D::Subscription),
            P::Invoice => Ok(D::Invoice),
            P::CreditNote => Ok(D::CreditNote),
            P::Quote => Ok(D::Quote),
            P::Signal => Ok(D::Signal),
            P::Unspecified => Err(ListViewsApiError::InvalidArgument(
                "entity_type is required".to_string(),
            )),
        }
    }

    fn entity_type_to_proto(t: EntityListType) -> i32 {
        use EntityListType as D;
        use server::EntityListType as P;
        let p = match t {
            D::Customer => P::Customer,
            D::Subscription => P::Subscription,
            D::Invoice => P::Invoice,
            D::CreditNote => P::CreditNote,
            D::Quote => P::Quote,
            D::Signal => P::Signal,
        };
        p as i32
    }

    pub fn parse_config(value: &str) -> Result<serde_json::Value, ListViewsApiError> {
        serde_json::from_str(value).map_err(|e| {
            ListViewsApiError::InvalidArgument(format!("invalid JSON in `config`: {e}"))
        })
    }

    pub fn entity_list_view_to_server(
        value: &domain::EntityListView,
        actor: UserId,
    ) -> server::EntityListView {
        server::EntityListView {
            id: value.id.as_proto(),
            name: value.name.clone(),
            entity_type: entity_type_to_proto(value.entity_type),
            config: value.config.to_string(),
            shared: value.shared,
            created_by: value.created_by.map(|u| u.as_proto()),
            created_by_name: value.created_by_name.clone(),
            is_owner: value.is_owned_by(actor),
            created_at: value.created_at.as_proto(),
            updated_at: value.updated_at.as_proto(),
        }
    }
}
