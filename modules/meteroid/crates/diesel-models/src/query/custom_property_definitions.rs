use crate::custom_property_definitions::CustomPropertyDefinitionRow;
use crate::enums::CustomPropertyEntityTypeEnum;
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};
use common_domain::ids::{CustomPropertyDefinitionId, TenantId};
use diesel::{ExpressionMethods, QueryDsl};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use tap::TapFallible;

impl CustomPropertyDefinitionRow {
    pub async fn get_by_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: CustomPropertyDefinitionId,
    ) -> DbResult<CustomPropertyDefinitionRow> {
        use crate::schema::custom_property_definition::dsl as t_dsl;

        let query = t_dsl::custom_property_definition
            .filter(t_dsl::id.eq(id))
            .filter(t_dsl::tenant_id.eq(tenant_id));
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while getting custom_property_definition")
            .into_db_result()
    }

    /// Lists definitions for a tenant, optionally filtered to one entity type. Archived
    /// definitions are excluded unless `include_archived` is set.
    pub async fn list(
        conn: &mut PgConn,
        tenant_id: TenantId,
        entity_type: Option<CustomPropertyEntityTypeEnum>,
        include_archived: bool,
    ) -> DbResult<Vec<CustomPropertyDefinitionRow>> {
        use crate::schema::custom_property_definition::dsl as t_dsl;

        let mut query = t_dsl::custom_property_definition
            .filter(t_dsl::tenant_id.eq(tenant_id))
            .into_boxed();

        if let Some(entity_type) = entity_type {
            query = query.filter(t_dsl::entity_type.eq(entity_type));
        }
        if !include_archived {
            query = query.filter(t_dsl::archived_at.is_null());
        }

        let query = query
            .order(t_dsl::display_order.asc())
            .then_order_by(t_dsl::created_at.asc());
        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .tap_err(|e| log::error!("Error while fetching custom_property_definitions: {e:?}"))
            .attach("Error while fetching custom_property_definitions")
            .into_db_result()
    }

    /// Active (non-archived) definitions for one entity type — the set enforced on writes.
    pub async fn list_active_for_entity(
        conn: &mut PgConn,
        tenant_id: TenantId,
        entity_type: CustomPropertyEntityTypeEnum,
    ) -> DbResult<Vec<CustomPropertyDefinitionRow>> {
        Self::list(conn, tenant_id, Some(entity_type), false).await
    }
}
