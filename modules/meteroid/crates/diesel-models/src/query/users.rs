use crate::errors::IntoDbResult;

use crate::users::{UserRow, UserRowNew, UserRowPatch, UserWithRoleRow};
use crate::{DbResult, PgConn};

use common_domain::ids::{OrganizationId, TenantId, UserId};
use diesel::{ExpressionMethods, JoinOnDsl, OptionalExtension, QueryDsl, SelectableHelper};
use error_stack::ResultExt;
use uuid::Uuid;

impl UserRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<()> {
        use crate::schema::user::dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(dsl::user).values(self);

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while inserting user")
            .into_db_result()
    }
}

impl UserRow {
    pub async fn find_by_id(conn: &mut PgConn, id: Uuid) -> DbResult<UserRow> {
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = u_dsl::user
            .filter(u_dsl::id.eq(id))
            .select(UserRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding user by id")
            .into_db_result()
    }

    pub async fn find_by_ids(conn: &mut PgConn, ids: &[UserId]) -> DbResult<Vec<UserRow>> {
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        if ids.is_empty() {
            return Ok(vec![]);
        }
        u_dsl::user
            .filter(u_dsl::id.eq_any(ids))
            .select(UserRow::as_select())
            .load(conn)
            .await
            .attach("Error while finding users by ids")
            .into_db_result()
    }

    pub async fn find_by_id_and_org_id(
        conn: &mut PgConn,
        id: Uuid,
        organization_id: OrganizationId,
    ) -> DbResult<UserWithRoleRow> {
        use crate::schema::organization_member::dsl as om_dsl;
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = u_dsl::user
            .inner_join(om_dsl::organization_member.on(u_dsl::id.eq(om_dsl::user_id)))
            .filter(u_dsl::id.eq(id))
            .filter(om_dsl::organization_id.eq(organization_id))
            .select(UserWithRoleRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding user by id and organization")
            .into_db_result()
    }

    pub async fn find_by_id_and_tenant_id(
        conn: &mut PgConn,
        id: Uuid,
        tenant_id: TenantId,
    ) -> DbResult<UserWithRoleRow> {
        use crate::schema::organization_member::dsl as om_dsl;
        use crate::schema::tenant::dsl as t_dsl; // we retrieve the org_id from the tenant table
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = u_dsl::user
            .inner_join(om_dsl::organization_member.on(u_dsl::id.eq(om_dsl::user_id)))
            .inner_join(t_dsl::tenant.on(om_dsl::organization_id.eq(t_dsl::organization_id)))
            .filter(u_dsl::id.eq(id))
            .filter(t_dsl::id.eq(tenant_id))
            .select(UserWithRoleRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding user by id and tenant")
            .into_db_result()
    }

    pub async fn find_by_email_and_org_id(
        conn: &mut PgConn,
        email: String,
        organization_id: OrganizationId,
    ) -> DbResult<UserWithRoleRow> {
        use crate::schema::organization_member::dsl as om_dsl;
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = u_dsl::user
            .inner_join(om_dsl::organization_member.on(u_dsl::id.eq(om_dsl::user_id)))
            .filter(u_dsl::email.eq(email))
            .filter(om_dsl::organization_id.eq(organization_id))
            .select(UserWithRoleRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while finding user by email")
            .into_db_result()
    }

    pub async fn find_by_email(conn: &mut PgConn, email: String) -> DbResult<Option<UserRow>> {
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = u_dsl::user
            .filter(u_dsl::email.eq(email))
            .select(UserRow::as_select());

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding user by email")
            .into_db_result()
    }

    pub async fn list_by_org_id(
        conn: &mut PgConn,
        organization_id: OrganizationId,
    ) -> DbResult<Vec<UserWithRoleRow>> {
        use crate::schema::organization_member::dsl as om_dsl;
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = u_dsl::user
            .inner_join(om_dsl::organization_member.on(u_dsl::id.eq(om_dsl::user_id)))
            .filter(om_dsl::organization_id.eq(organization_id))
            .order_by((om_dsl::role.asc(), u_dsl::email.asc()))
            .select(UserWithRoleRow::as_select());

        crate::debug_sql(&query);

        query
            .get_results(conn)
            .await
            .attach("Error while listing users")
            .into_db_result()
    }

    pub async fn any_exists(conn: &mut PgConn) -> DbResult<bool> {
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::dsl::select(diesel::dsl::exists(u_dsl::user.limit(1)));

        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .attach("Error while checking if any user exists")
            .into_db_result()
    }

    pub async fn update_password_hash(
        conn: &mut PgConn,
        user_id: Uuid,
        new_password_hash: &str,
    ) -> DbResult<()> {
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(u_dsl::user)
            .filter(u_dsl::id.eq(user_id))
            .set(u_dsl::password_hash.eq(new_password_hash));

        crate::debug_sql(&query);

        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while updating user password")
            .into_db_result()
    }

    pub async fn set_pending_totp_secret(
        conn: &mut PgConn,
        user_id: UserId,
        secret: &str,
    ) -> DbResult<()> {
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        diesel::update(u_dsl::user)
            .filter(u_dsl::id.eq(user_id))
            .filter(u_dsl::totp_enabled.eq(false))
            .set(u_dsl::totp_secret.eq(secret))
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while setting pending TOTP secret")
            .into_db_result()
    }

    pub async fn activate_totp(
        conn: &mut PgConn,
        user_id: UserId,
        recovery_code_hashes: &[String],
    ) -> DbResult<()> {
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        diesel::update(u_dsl::user)
            .filter(u_dsl::id.eq(user_id))
            .set((
                u_dsl::totp_enabled.eq(true),
                u_dsl::totp_recovery_codes.eq(recovery_code_hashes),
                u_dsl::totp_enrolled_at.eq(diesel::dsl::now),
            ))
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while activating TOTP")
            .into_db_result()
    }

    pub async fn disable_totp(conn: &mut PgConn, user_id: UserId) -> DbResult<()> {
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        diesel::update(u_dsl::user)
            .filter(u_dsl::id.eq(user_id))
            .set((
                u_dsl::totp_secret.eq::<Option<String>>(None),
                u_dsl::totp_enabled.eq(false),
                u_dsl::totp_recovery_codes.eq::<Vec<String>>(vec![]),
                u_dsl::totp_enrolled_at.eq::<Option<chrono::NaiveDateTime>>(None),
            ))
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while disabling TOTP")
            .into_db_result()
    }

    pub async fn replace_recovery_codes(
        conn: &mut PgConn,
        user_id: UserId,
        recovery_code_hashes: &[String],
    ) -> DbResult<()> {
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        diesel::update(u_dsl::user)
            .filter(u_dsl::id.eq(user_id))
            .filter(u_dsl::totp_enabled.eq(true))
            .set(u_dsl::totp_recovery_codes.eq(recovery_code_hashes))
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while replacing recovery codes")
            .into_db_result()
    }

    pub async fn consume_recovery_code(
        conn: &mut PgConn,
        user_id: UserId,
        remaining_hashes: &[String],
    ) -> DbResult<usize> {
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        diesel::update(u_dsl::user)
            .filter(u_dsl::id.eq(user_id))
            .filter(u_dsl::totp_enabled.eq(true))
            .set(u_dsl::totp_recovery_codes.eq(remaining_hashes))
            .execute(conn)
            .await
            .attach("Error while consuming recovery code")
            .into_db_result()
    }

    /// Scrubs every personal field and archives the row, keeping only the id that business
    /// records still reference. Returns 0 when the user is unknown or already archived.
    pub async fn anonymize(conn: &mut PgConn, user_id: UserId) -> DbResult<usize> {
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        diesel::update(u_dsl::user)
            .filter(u_dsl::id.eq(user_id))
            .filter(u_dsl::archived_at.is_null())
            .set((
                u_dsl::email.eq(format!("deleted+{user_id}@deleted.invalid")),
                u_dsl::first_name.eq::<Option<String>>(None),
                u_dsl::last_name.eq::<Option<String>>(None),
                u_dsl::department.eq::<Option<String>>(None),
                u_dsl::password_hash.eq::<Option<String>>(None),
                u_dsl::totp_secret.eq::<Option<String>>(None),
                u_dsl::totp_enabled.eq(false),
                u_dsl::totp_recovery_codes.eq::<Vec<String>>(vec![]),
                u_dsl::totp_enrolled_at.eq::<Option<chrono::NaiveDateTime>>(None),
                u_dsl::preferences.eq(serde_json::json!({})),
                u_dsl::archived_at.eq(diesel::dsl::now),
            ))
            .execute(conn)
            .await
            .attach("Error while anonymizing user")
            .into_db_result()
    }

    pub async fn is_active(conn: &mut PgConn, user_id: UserId) -> DbResult<bool> {
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        diesel::select(diesel::dsl::exists(
            u_dsl::user
                .filter(u_dsl::id.eq(user_id))
                .filter(u_dsl::archived_at.is_null()),
        ))
        .get_result(conn)
        .await
        .attach("Error while checking whether user is active")
        .into_db_result()
    }

    /// Shallow-merges `patch` into the user's `preferences` (`preferences || patch`),
    /// so concurrent writers of different top-level keys don't clobber each other,
    /// and returns the merged object.
    pub async fn merge_preferences(
        conn: &mut PgConn,
        user_id: UserId,
        patch: serde_json::Value,
    ) -> DbResult<serde_json::Value> {
        use crate::schema::user::dsl as u_dsl;
        use diesel::PgJsonbExpressionMethods;
        use diesel_async::RunQueryDsl;

        diesel::update(u_dsl::user)
            .filter(u_dsl::id.eq(user_id))
            .set(u_dsl::preferences.eq(u_dsl::preferences.concat(patch)))
            .returning(u_dsl::preferences)
            .get_result(conn)
            .await
            .attach("Error while updating user preferences")
            .into_db_result()
    }
}

impl UserRowPatch {
    pub async fn update_user(&self, conn: &mut PgConn) -> DbResult<UserRow> {
        use crate::schema::user::dsl as u_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(u_dsl::user)
            .filter(u_dsl::id.eq(self.id))
            .set(self);

        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while updating user")
            .into_db_result()
    }
}
