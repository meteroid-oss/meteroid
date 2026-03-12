use crate::errors::IntoDbResult;
use crate::subscription_add_ons::{SubscriptionAddOnRow, SubscriptionAddOnRowNew};
use crate::{DbResult, PgConn};
use common_domain::ids::{SubscriptionAddOnId, SubscriptionId, TenantId};
use diesel::{ExpressionMethods, OptionalExtension, SelectableHelper};
use diesel::{QueryDsl, debug_query};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl SubscriptionAddOnRow {
    pub async fn insert_batch(
        conn: &mut PgConn,
        batch: Vec<&SubscriptionAddOnRowNew>,
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        let query = diesel::insert_into(sao_dsl::subscription_add_on).values(batch);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while inserting SubscriptionAddOn batch")
            .into_db_result()
    }

    pub async fn list_by_subscription_id(
        conn: &mut PgConn,
        tenant_id: &TenantId,
        subscription_id: &SubscriptionId,
    ) -> DbResult<Vec<SubscriptionAddOnRow>> {
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        let query = sao_dsl::subscription_add_on
            .inner_join(s_dsl::subscription)
            .filter(sao_dsl::subscription_id.eq(subscription_id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .select(SubscriptionAddOnRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        query
            .get_results(conn)
            .await
            .attach("Error while listing SubscriptionAddOn by subscription_id")
            .into_db_result()
    }

    pub async fn delete_by_id(
        conn: &mut PgConn,
        id: SubscriptionAddOnId,
        subscription_id: &SubscriptionId,
        tenant_id: &TenantId,
    ) -> DbResult<()> {
        use crate::errors::{DatabaseError, DatabaseErrorContainer};
        use crate::schema::subscription::dsl as s_dsl;
        use crate::schema::subscription_add_on::dsl as sao_dsl;

        // Verify the subscription belongs to the tenant
        let sub_exists = s_dsl::subscription
            .filter(s_dsl::id.eq(subscription_id))
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .select(s_dsl::id)
            .first::<SubscriptionId>(conn)
            .await
            .optional()
            .attach("Error verifying subscription tenant")
            .into_db_result()?;

        if sub_exists.is_none() {
            return Err(DatabaseErrorContainer::from(DatabaseError::NotFound));
        }

        let query = diesel::delete(
            sao_dsl::subscription_add_on
                .filter(sao_dsl::id.eq(id))
                .filter(sao_dsl::subscription_id.eq(subscription_id)),
        );

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));

        let affected = query
            .execute(conn)
            .await
            .attach("Error while deleting SubscriptionAddOn")
            .into_db_result()?;

        if affected == 0 {
            return Err(DatabaseErrorContainer::from(DatabaseError::NotFound));
        }

        Ok(())
    }
}
