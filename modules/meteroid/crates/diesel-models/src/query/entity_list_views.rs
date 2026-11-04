//! Saved list views. Every read and write is scoped to what `actor` may see — the tenant's shared
//! views plus the actor's own private ones — directly in SQL, so a view id alone never grants
//! access and a hidden view is indistinguishable from a missing one.

use crate::entity_list_views::{
    EntityListViewDefaultRow, EntityListViewRow, EntityListViewRowNew, EntityListViewRowPatch,
};
use crate::enums::EntityListTypeEnum;
use crate::errors::IntoDbResult;
use crate::{DbResult, PgConn};

use common_domain::ids::{EntityListViewId, TenantId, UserId};
use diesel::dsl::count_star;
use diesel::pg::Pg;
use diesel::prelude::{
    BoolExpressionMethods, ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper,
};
use diesel::{BoxableExpression, sql_types};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

use crate::schema::entity_list_view::dsl as v;
use crate::schema::entity_list_view_default::dsl as d;

type VisiblePredicate = Box<
    dyn BoxableExpression<v::entity_list_view, Pg, SqlType = sql_types::Nullable<sql_types::Bool>>,
>;

/// `tenant = $tenant AND (shared OR created_by = $actor)`.
fn visible_to(tenant: TenantId, actor: UserId) -> VisiblePredicate {
    Box::new(
        v::tenant_id
            .eq(tenant)
            .and(v::shared.eq(true).or(v::created_by.eq(actor))),
    )
}

impl EntityListViewRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<EntityListViewRow> {
        let query = diesel::insert_into(v::entity_list_view).values(self);

        crate::debug_sql(&query);

        query
            .returning(EntityListViewRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while inserting entity list view")
            .into_db_result()
    }
}

impl EntityListViewRow {
    /// Views of `entity` visible to `actor`, oldest first (a stable tab order).
    pub async fn list_visible(
        conn: &mut PgConn,
        tenant: TenantId,
        actor: UserId,
        entity: EntityListTypeEnum,
    ) -> DbResult<Vec<EntityListViewRow>> {
        let query = v::entity_list_view
            .filter(visible_to(tenant, actor))
            .filter(v::entity_type.eq(entity))
            .order((v::created_at.asc(), v::id.asc()))
            .select(EntityListViewRow::as_select());

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing entity list views")
            .into_db_result()
    }

    /// A view visible to `actor`; `NotFound` otherwise.
    pub async fn get_visible(
        conn: &mut PgConn,
        id: EntityListViewId,
        tenant: TenantId,
        actor: UserId,
    ) -> DbResult<EntityListViewRow> {
        v::entity_list_view
            .filter(v::id.eq(id))
            .filter(visible_to(tenant, actor))
            .select(EntityListViewRow::as_select())
            .first(conn)
            .await
            .attach("Error while finding entity list view")
            .into_db_result()
    }

    /// Same as [`Self::get_visible`] but row-locked, for read-check-write in a transaction.
    pub async fn get_visible_for_update(
        conn: &mut PgConn,
        id: EntityListViewId,
        tenant: TenantId,
        actor: UserId,
    ) -> DbResult<EntityListViewRow> {
        v::entity_list_view
            .filter(v::id.eq(id))
            .filter(visible_to(tenant, actor))
            .select(EntityListViewRow::as_select())
            .for_update()
            .first(conn)
            .await
            .attach("Error while locking entity list view")
            .into_db_result()
    }

    /// Number of views `creator` owns for `entity` (quota enforcement).
    pub async fn count_owned(
        conn: &mut PgConn,
        tenant: TenantId,
        creator: UserId,
        entity: EntityListTypeEnum,
    ) -> DbResult<i64> {
        v::entity_list_view
            .filter(v::tenant_id.eq(tenant))
            .filter(v::created_by.eq(creator))
            .filter(v::entity_type.eq(entity))
            .select(count_star())
            .get_result(conn)
            .await
            .attach("Error while counting entity list views")
            .into_db_result()
    }

    pub async fn patch_visible(
        conn: &mut PgConn,
        id: EntityListViewId,
        tenant: TenantId,
        actor: UserId,
        patch: &EntityListViewRowPatch,
    ) -> DbResult<EntityListViewRow> {
        let query = diesel::update(v::entity_list_view)
            .filter(v::id.eq(id))
            .filter(visible_to(tenant, actor))
            .set(patch);

        crate::debug_sql(&query);

        query
            .returning(EntityListViewRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while updating entity list view")
            .into_db_result()
    }

    /// Deletes a view visible to `actor`. Returns the number of rows removed (0 = not found).
    pub async fn delete_visible(
        conn: &mut PgConn,
        id: EntityListViewId,
        tenant: TenantId,
        actor: UserId,
    ) -> DbResult<usize> {
        let query = diesel::delete(v::entity_list_view)
            .filter(v::id.eq(id))
            .filter(visible_to(tenant, actor));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting entity list view")
            .into_db_result()
    }
}

impl EntityListViewDefaultRow {
    /// `actor`'s default view for `entity`, if any. The FK cascade guarantees it still exists; it
    /// is re-checked for visibility since a shared default may since have been made private.
    pub async fn get_visible(
        conn: &mut PgConn,
        tenant: TenantId,
        actor: UserId,
        entity: EntityListTypeEnum,
    ) -> DbResult<Option<EntityListViewId>> {
        d::entity_list_view_default
            .inner_join(v::entity_list_view)
            .filter(d::tenant_id.eq(tenant))
            .filter(d::user_id.eq(actor))
            .filter(d::entity_type.eq(entity))
            .filter(v::shared.eq(true).or(v::created_by.eq(actor)))
            .select(d::view_id)
            .first(conn)
            .await
            .optional()
            .attach("Error while finding default entity list view")
            .into_db_result()
    }

    pub async fn upsert(&self, conn: &mut PgConn) -> DbResult<()> {
        diesel::insert_into(d::entity_list_view_default)
            .values(self)
            .on_conflict((d::tenant_id, d::user_id, d::entity_type))
            .do_update()
            .set(d::view_id.eq(self.view_id))
            .execute(conn)
            .await
            .attach("Error while setting default entity list view")
            .into_db_result()
            .map(|_| ())
    }

    /// Removes every other user's default pointing at `view_id` (the view was made private, so
    /// only `keep` — its creator — can still see it).
    pub async fn clear_for_view_except(
        conn: &mut PgConn,
        view_id: EntityListViewId,
        keep: UserId,
    ) -> DbResult<()> {
        let query = diesel::delete(d::entity_list_view_default)
            .filter(d::view_id.eq(view_id))
            .filter(d::user_id.ne(keep));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while clearing defaults of entity list view")
            .into_db_result()
            .map(|_| ())
    }

    pub async fn clear(
        conn: &mut PgConn,
        tenant: TenantId,
        actor: UserId,
        entity: EntityListTypeEnum,
    ) -> DbResult<()> {
        diesel::delete(d::entity_list_view_default)
            .filter(d::tenant_id.eq(tenant))
            .filter(d::user_id.eq(actor))
            .filter(d::entity_type.eq(entity))
            .execute(conn)
            .await
            .attach("Error while clearing default entity list view")
            .into_db_result()
            .map(|_| ())
    }

    /// Drops a departed user's defaults and private views; their shared views stay with the tenant.
    pub async fn delete_all_private_for_user(conn: &mut PgConn, user: UserId) -> DbResult<()> {
        diesel::delete(d::entity_list_view_default)
            .filter(d::user_id.eq(user))
            .execute(conn)
            .await
            .attach("Error while deleting a user's default entity list views")
            .into_db_result()?;

        diesel::delete(v::entity_list_view)
            .filter(v::created_by.eq(user))
            .filter(v::shared.eq(false))
            .execute(conn)
            .await
            .attach("Error while deleting a user's private entity list views")
            .into_db_result()
            .map(|_| ())
    }
}
