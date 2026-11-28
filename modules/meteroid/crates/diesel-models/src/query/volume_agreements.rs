use chrono::NaiveDate;
use common_domain::ids::{SubscriptionId, TenantId, VolumeAgreementId};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

use crate::schema::{
    volume_agreement as va, volume_agreement_basis as vab, volume_agreement_member as vam,
    volume_agreement_metric as vamt,
};
use crate::volume_agreements::{
    VolumeAgreementBasisRow, VolumeAgreementMemberRow, VolumeAgreementMetricRow, VolumeAgreementRow,
};
use crate::{DbResult, PgConn};

impl VolumeAgreementRow {
    pub async fn find(
        conn: &mut PgConn,
        tenant_id: TenantId,
        id: VolumeAgreementId,
    ) -> DbResult<VolumeAgreementRow> {
        va::table
            .filter(va::tenant_id.eq(tenant_id))
            .filter(va::id.eq(id))
            .select(VolumeAgreementRow::as_select())
            .first(conn)
            .await
            .map_err(Into::into)
    }
}

impl VolumeAgreementMemberRow {
    /// The subscription's memberships, with their agreements, oldest first.
    pub async fn memberships_of(
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> DbResult<Vec<(VolumeAgreementMemberRow, VolumeAgreementRow)>> {
        vam::table
            .inner_join(va::table.on(va::id.eq(vam::agreement_id)))
            .filter(vam::tenant_id.eq(tenant_id))
            .filter(vam::subscription_id.eq(subscription_id))
            .order(vam::from_date.asc())
            .select((
                VolumeAgreementMemberRow::as_select(),
                VolumeAgreementRow::as_select(),
            ))
            .load(conn)
            .await
            .map_err(Into::into)
    }

    pub async fn list_of(
        conn: &mut PgConn,
        tenant_id: TenantId,
        agreement_id: VolumeAgreementId,
    ) -> DbResult<Vec<VolumeAgreementMemberRow>> {
        vam::table
            .filter(vam::tenant_id.eq(tenant_id))
            .filter(vam::agreement_id.eq(agreement_id))
            .order((vam::from_date.asc(), vam::id.asc()))
            .select(VolumeAgreementMemberRow::as_select())
            .load(conn)
            .await
            .map_err(Into::into)
    }

    /// The agreement's members whose membership covers the window `[from, until)`.
    pub async fn covering(
        conn: &mut PgConn,
        tenant_id: TenantId,
        agreement_id: VolumeAgreementId,
        from: NaiveDate,
        until: NaiveDate,
    ) -> DbResult<Vec<VolumeAgreementMemberRow>> {
        vam::table
            .filter(vam::tenant_id.eq(tenant_id))
            .filter(vam::agreement_id.eq(agreement_id))
            .filter(vam::from_date.le(from))
            .filter(vam::until_date.is_null().or(vam::until_date.ge(until)))
            .order(vam::id.asc())
            .select(VolumeAgreementMemberRow::as_select())
            .load(conn)
            .await
            .map_err(Into::into)
    }
}

impl VolumeAgreementMemberRow {
    /// The longest grace of the invoicing entities of the members covering `[from, until)`.
    pub async fn longest_grace_hours(
        conn: &mut PgConn,
        tenant_id: TenantId,
        agreement_id: VolumeAgreementId,
        from: NaiveDate,
        until: NaiveDate,
    ) -> DbResult<Option<i32>> {
        use crate::schema::{customer as c, invoicing_entity as ie, subscription as s};
        vam::table
            .inner_join(s::table.on(s::id.eq(vam::subscription_id)))
            .inner_join(c::table.on(c::id.eq(s::customer_id)))
            .inner_join(ie::table.on(ie::id.eq(c::invoicing_entity_id)))
            .filter(vam::tenant_id.eq(tenant_id))
            .filter(vam::agreement_id.eq(agreement_id))
            .filter(vam::from_date.le(from))
            .filter(vam::until_date.is_null().or(vam::until_date.ge(until)))
            .select(diesel::dsl::max(ie::grace_period_hours))
            .first(conn)
            .await
            .map_err(Into::into)
    }
}

impl VolumeAgreementMetricRow {
    pub async fn list_of(
        conn: &mut PgConn,
        agreement_ids: &[VolumeAgreementId],
    ) -> DbResult<Vec<VolumeAgreementMetricRow>> {
        vamt::table
            .filter(vamt::agreement_id.eq_any(agreement_ids))
            .select(VolumeAgreementMetricRow::as_select())
            .load(conn)
            .await
            .map_err(Into::into)
    }
}

impl VolumeAgreementBasisRow {
    pub async fn of_agreement(
        conn: &mut PgConn,
        agreement_id: VolumeAgreementId,
    ) -> DbResult<Vec<VolumeAgreementBasisRow>> {
        vab::table
            .filter(vab::agreement_id.eq(agreement_id))
            .order((vab::period_start.asc(), vab::billable_metric_id.asc()))
            .select(VolumeAgreementBasisRow::as_select())
            .load(conn)
            .await
            .map_err(Into::into)
    }

    /// The closed bases of the agreement's window starting `period_start`, one per pooled metric.
    pub async fn of_window(
        conn: &mut PgConn,
        agreement_id: VolumeAgreementId,
        period_start: NaiveDate,
    ) -> DbResult<Vec<VolumeAgreementBasisRow>> {
        vab::table
            .filter(vab::agreement_id.eq(agreement_id))
            .filter(vab::period_start.eq(period_start))
            .select(VolumeAgreementBasisRow::as_select())
            .load(conn)
            .await
            .map_err(Into::into)
    }

    /// Writes a window's bases; a basis already written stays as it is.
    pub async fn insert_new(
        conn: &mut PgConn,
        rows: &[VolumeAgreementBasisRow],
    ) -> DbResult<usize> {
        diesel::insert_into(vab::table)
            .values(rows)
            .on_conflict_do_nothing()
            .execute(conn)
            .await
            .map_err(Into::into)
    }

    /// Serializes the close of the agreement's window starting `period_start`, until the
    /// transaction ends.
    pub async fn lock_window(
        conn: &mut PgConn,
        agreement_id: VolumeAgreementId,
        period_start: NaiveDate,
    ) -> DbResult<()> {
        diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
            .bind::<diesel::sql_types::Text, _>(format!(
                "volume-agreement-window:{agreement_id}:{period_start}"
            ))
            .execute(conn)
            .await
            .map(drop)
            .map_err(Into::into)
    }
}
