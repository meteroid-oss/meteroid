use chrono::{DateTime, Utc};
use diesel::{
    ExpressionMethods, JoinOnDsl, OptionalExtension, QueryDsl, SelectableHelper, debug_query,
};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;
use tap::TapFallible;

use common_domain::ids::{OrganizationId, OrganizationInviteId};

use crate::errors::IntoDbResult;
use crate::organization_invites::{OrganizationInviteRow, OrganizationInviteRowNew};
use crate::{DbResult, PgConn};

impl OrganizationInviteRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<OrganizationInviteRow> {
        use crate::schema::organization_invite::dsl::organization_invite;
        let query = diesel::insert_into(organization_invite).values(self);
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));
        query
            .get_result(conn)
            .await
            .attach("Error inserting organization_invite")
            .into_db_result()
    }
}

impl OrganizationInviteRow {
    pub async fn find_by_id(
        conn: &mut PgConn,
        id: OrganizationInviteId,
    ) -> DbResult<OrganizationInviteRow> {
        use crate::schema::organization_invite::dsl as oi_dsl;
        let query = oi_dsl::organization_invite.filter(oi_dsl::id.eq(id));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));
        query
            .first(conn)
            .await
            .attach("Error finding organization_invite by id")
            .into_db_result()
    }

    pub async fn find_pending_by_email_and_org(
        conn: &mut PgConn,
        org_id: OrganizationId,
        email: &str,
    ) -> DbResult<Option<OrganizationInviteRow>> {
        use crate::schema::organization_invite::dsl as oi_dsl;
        let now = Utc::now();
        // emails are normalized to lowercase on insert, so plain eq() suffices
        let query = oi_dsl::organization_invite
            .filter(oi_dsl::organization_id.eq(org_id))
            .filter(oi_dsl::invited_email.eq(email.to_lowercase()))
            .filter(oi_dsl::accepted_at.is_null())
            .filter(oi_dsl::revoked_at.is_null())
            .filter(oi_dsl::expires_at.gt(now));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));
        query
            .first(conn)
            .await
            .optional()
            .attach("Error finding pending invite by email")
            .into_db_result()
    }

    pub async fn update_expires_at(
        conn: &mut PgConn,
        id: OrganizationInviteId,
        new_expires_at: DateTime<Utc>,
    ) -> DbResult<()> {
        use crate::schema::organization_invite::dsl as oi_dsl;
        let query = diesel::update(oi_dsl::organization_invite)
            .filter(oi_dsl::id.eq(id))
            .set(oi_dsl::expires_at.eq(new_expires_at));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));
        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error updating invite expires_at: {e:?}"))
            .attach("Error updating invite expires_at")
            .into_db_result()
            .map(|_| ())
    }

    pub async fn set_accepted_at(
        conn: &mut PgConn,
        id: OrganizationInviteId,
        accepted_at: DateTime<Utc>,
    ) -> DbResult<()> {
        use crate::schema::organization_invite::dsl as oi_dsl;
        let query = diesel::update(oi_dsl::organization_invite)
            .filter(oi_dsl::id.eq(id))
            .set(oi_dsl::accepted_at.eq(accepted_at));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));
        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error setting invite accepted_at: {e:?}"))
            .attach("Error setting invite accepted_at")
            .into_db_result()
            .map(|_| ())
    }

    pub async fn set_revoked_at(
        conn: &mut PgConn,
        id: OrganizationInviteId,
        revoked_at: DateTime<Utc>,
    ) -> DbResult<()> {
        use crate::schema::organization_invite::dsl as oi_dsl;
        let query = diesel::update(oi_dsl::organization_invite)
            .filter(oi_dsl::id.eq(id))
            .set(oi_dsl::revoked_at.eq(revoked_at));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));
        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error setting invite revoked_at: {e:?}"))
            .attach("Error setting invite revoked_at")
            .into_db_result()
            .map(|_| ())
    }

    pub async fn find_with_inviter_email(
        conn: &mut PgConn,
        id: OrganizationInviteId,
    ) -> DbResult<(OrganizationInviteRow, String)> {
        use crate::schema::organization_invite::dsl as oi_dsl;
        use crate::schema::user::dsl as u_dsl;
        let result: (OrganizationInviteRow, String) = oi_dsl::organization_invite
            .inner_join(u_dsl::user.on(oi_dsl::invited_by.eq(u_dsl::id)))
            .filter(oi_dsl::id.eq(id))
            .select((OrganizationInviteRow::as_select(), u_dsl::email))
            .first(conn)
            .await
            .attach("Error finding invite with inviter email")
            .into_db_result()?;
        Ok(result)
    }

    pub async fn revoke_expired_for_email_and_org(
        conn: &mut PgConn,
        org_id: OrganizationId,
        email: &str,
    ) -> DbResult<()> {
        use crate::schema::organization_invite::dsl as oi_dsl;
        let now = Utc::now();
        let query = diesel::update(oi_dsl::organization_invite)
            .filter(oi_dsl::organization_id.eq(org_id))
            .filter(oi_dsl::invited_email.eq(email.to_lowercase()))
            .filter(oi_dsl::accepted_at.is_null())
            .filter(oi_dsl::revoked_at.is_null())
            .filter(oi_dsl::expires_at.le(now))
            .set(oi_dsl::revoked_at.eq(now));
        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query));
        query
            .execute(conn)
            .await
            .tap_err(|e| log::error!("Error revoking expired invites: {e:?}"))
            .attach("Error revoking expired invites")
            .into_db_result()
            .map(|_| ())
    }

    pub async fn list_pending_with_inviter_email(
        conn: &mut PgConn,
        org_id: OrganizationId,
    ) -> DbResult<Vec<(OrganizationInviteRow, String)>> {
        use crate::schema::organization_invite::dsl as oi_dsl;
        use crate::schema::user::dsl as u_dsl;
        oi_dsl::organization_invite
            .inner_join(u_dsl::user.on(oi_dsl::invited_by.eq(u_dsl::id)))
            .filter(oi_dsl::organization_id.eq(org_id))
            .filter(oi_dsl::accepted_at.is_null())
            .filter(oi_dsl::revoked_at.is_null())
            .order(oi_dsl::created_at.desc())
            .select((OrganizationInviteRow::as_select(), u_dsl::email))
            .get_results(conn)
            .await
            .attach("Error listing pending invites with inviter")
            .into_db_result()
    }
}
