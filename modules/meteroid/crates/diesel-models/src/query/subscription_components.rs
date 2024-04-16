use crate::errors::IntoDbResult;
use std::collections::HashMap;

use crate::subscription_components::{SubscriptionComponent, SubscriptionComponentNew};
use crate::{DbResult, PgConn};

use diesel::debug_query;
use error_stack::ResultExt;

use diesel::{ExpressionMethods, QueryDsl, SelectableHelper};
use itertools::Itertools;

impl SubscriptionComponentNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SubscriptionComponent> {
        use crate::schema::subscription_component::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(subscription_component).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting SubscriptionComponent")
            .into_db_result()
    }
}

impl SubscriptionComponent {
    pub async fn insert_subscription_component_batch(
        conn: &mut PgConn,
        batch: Vec<&SubscriptionComponentNew>,
    ) -> DbResult<Vec<SubscriptionComponent>> {
        use crate::schema::subscription_component::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(subscription_component).values(batch);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while inserting SubscriptionComponent batch")
            .into_db_result()
    }

    pub async fn list_subscription_components_by_subscription(
        conn: &mut PgConn,
        tenant_id_params: &uuid::Uuid,
        subscription_id: &uuid::Uuid,
    ) -> DbResult<Vec<SubscriptionComponent>> {
        use crate::schema::subscription_component::dsl as subscription_component_dsl;
        use diesel_async::RunQueryDsl;

        let query = subscription_component_dsl::subscription_component
            .inner_join(crate::schema::subscription::table)
            .filter(subscription_component_dsl::subscription_id.eq(subscription_id))
            .filter(crate::schema::subscription::tenant_id.eq(tenant_id_params))
            .select(SubscriptionComponent::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching SubscriptionComponents by subscription")
            .into_db_result()
    }

    pub async fn list_subscription_components_by_subscriptions(
        conn: &mut PgConn,
        tenant_id_params: &uuid::Uuid,
        subscription_ids: &[uuid::Uuid],
    ) -> DbResult<HashMap<uuid::Uuid, Vec<SubscriptionComponent>>> {
        use crate::schema::subscription_component::dsl as subscription_component_dsl;
        use diesel_async::RunQueryDsl;

        let query = subscription_component_dsl::subscription_component
            .filter(subscription_component_dsl::subscription_id.eq_any(subscription_ids))
            .inner_join(crate::schema::subscription::table)
            .filter(crate::schema::subscription::tenant_id.eq(tenant_id_params))
            .select(SubscriptionComponent::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        let res: Vec<SubscriptionComponent> = query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching SubscriptionComponents by subscriptions")
            .into_db_result()?;

        let grouped = res.into_iter().into_group_map_by(|c| c.subscription_id);
        Ok(grouped)
    }
}
