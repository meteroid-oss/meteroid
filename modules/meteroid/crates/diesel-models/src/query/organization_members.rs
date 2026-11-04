use crate::errors::IntoDbResult;
use crate::organization_members::OrganizationMemberRow;

use crate::{DbResult, PgConn};

use common_domain::ids::OrganizationId;
use diesel::ExpressionMethods;
use diesel::QueryDsl;
use error_stack::ResultExt;
use uuid::Uuid;

impl OrganizationMemberRow {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<OrganizationMemberRow> {
        use crate::schema::organization_member::dsl::organization_member;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(organization_member).values(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting organization member")
            .into_db_result()
    }

    /// Whether `user_id_param` is a member of `org_id_param`, share-locking the membership until
    /// the transaction ends: a removal then waits for the caller's write, or the caller sees it.
    /// Customers are tenant-scoped and members org-scoped, so no FK can express membership.
    pub async fn lock_membership(
        conn: &mut PgConn,
        user_id_param: Uuid,
        org_id_param: OrganizationId,
    ) -> DbResult<bool> {
        use crate::schema::organization_member::dsl::{
            organization_id, organization_member, user_id,
        };
        use diesel::OptionalExtension;
        use diesel_async::RunQueryDsl;

        let query = organization_member
            .filter(user_id.eq(user_id_param))
            .filter(organization_id.eq(org_id_param))
            .select(user_id)
            .for_share();

        let found: Option<Uuid> = query
            .first(conn)
            .await
            .optional()
            .attach("Error while checking organization membership")
            .into_db_result()?;

        Ok(found.is_some())
    }

    /// The subset of `user_ids` that are members of `org_id_param`.
    pub async fn filter_members(
        conn: &mut PgConn,
        org_id_param: OrganizationId,
        user_ids: &[Uuid],
    ) -> DbResult<Vec<Uuid>> {
        use crate::schema::organization_member::dsl::{
            organization_id, organization_member, user_id,
        };
        use diesel_async::RunQueryDsl;

        if user_ids.is_empty() {
            return Ok(vec![]);
        }
        organization_member
            .filter(organization_id.eq(org_id_param))
            .filter(user_id.eq_any(user_ids))
            .select(user_id)
            .load(conn)
            .await
            .attach("Error while filtering organization members")
            .into_db_result()
    }

    pub async fn find_by_user_and_org(
        conn: &mut PgConn,
        user_id_param: Uuid,
        org_id_param: OrganizationId,
    ) -> DbResult<OrganizationMemberRow> {
        use crate::schema::organization_member::dsl::{
            organization_id, organization_member, user_id,
        };
        use diesel_async::RunQueryDsl;

        let query = organization_member
            .filter(user_id.eq(user_id_param))
            .filter(organization_id.eq(org_id_param));

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding organization member")
            .into_db_result()
    }

    pub async fn delete_member(
        conn: &mut PgConn,
        user_id_param: Uuid,
        org_id_param: OrganizationId,
    ) -> DbResult<()> {
        use crate::schema::organization_member::dsl::{
            organization_id, organization_member, user_id,
        };
        use diesel_async::RunQueryDsl;

        let query = diesel::delete(
            organization_member
                .filter(user_id.eq(user_id_param))
                .filter(organization_id.eq(org_id_param)),
        );

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while deleting organization member")
            .into_db_result()?;

        Ok(())
    }

    pub async fn update_role(
        conn: &mut PgConn,
        user_id_param: Uuid,
        org_id_param: OrganizationId,
        new_role: crate::enums::OrganizationUserRole,
        new_role_id: Option<common_domain::ids::RoleId>,
    ) -> DbResult<()> {
        use crate::schema::organization_member::dsl::{
            organization_id, organization_member, role, role_id, user_id,
        };
        use diesel_async::RunQueryDsl;

        let query = diesel::update(
            organization_member
                .filter(user_id.eq(user_id_param))
                .filter(organization_id.eq(org_id_param)),
        )
        .set((role.eq(new_role), role_id.eq(new_role_id)));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .attach("Error while updating organization member role")
            .into_db_result()
            .map(|_| ())
    }

    pub async fn count_admins(conn: &mut PgConn, org_id_param: OrganizationId) -> DbResult<i64> {
        use crate::enums::OrganizationUserRole;
        use crate::schema::organization_member::dsl::{organization_id, organization_member, role};
        use diesel::dsl::count_star;
        use diesel_async::RunQueryDsl;

        let query = organization_member
            .filter(organization_id.eq(org_id_param))
            .filter(role.eq(OrganizationUserRole::Admin))
            .select(count_star());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while counting admins in organization")
            .into_db_result()
    }
    pub async fn list_by_user_for_update(
        conn: &mut PgConn,
        user_id_param: Uuid,
    ) -> DbResult<Vec<OrganizationMemberRow>> {
        use crate::schema::organization_member::dsl::{organization_member, user_id};
        use diesel_async::RunQueryDsl;

        let query = organization_member
            .filter(user_id.eq(user_id_param))
            .for_update();

        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing memberships of user")
            .into_db_result()
    }

    pub async fn count_members(conn: &mut PgConn, org_id_param: OrganizationId) -> DbResult<i64> {
        use crate::schema::organization_member::dsl::{organization_id, organization_member};
        use diesel::dsl::count_star;
        use diesel_async::RunQueryDsl;

        organization_member
            .filter(organization_id.eq(org_id_param))
            .select(count_star())
            .first(conn)
            .await
            .attach("Error while counting members in organization")
            .into_db_result()
    }
}
