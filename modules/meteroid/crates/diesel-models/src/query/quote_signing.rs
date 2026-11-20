use crate::errors::IntoDbResult;
use crate::quote_signing::{
    QuoteEvidenceEventRow, QuoteEvidenceEventRowNew, QuoteSignatureCertificateRow,
    QuoteSignatureOtpRow, QuoteSignatureOtpRowNew,
};
use crate::{DbResult, PgConn};
use common_domain::ids::{QuoteId, QuoteSignatureOtpId, TenantId};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

impl QuoteEvidenceEventRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<QuoteEvidenceEventRow> {
        use crate::schema::quote_evidence_event::dsl as d;

        let query = diesel::insert_into(d::quote_evidence_event).values(self);
        crate::debug_sql(&query);
        query
            .returning(QuoteEvidenceEventRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while inserting quote evidence event")
            .into_db_result()
    }
}

impl QuoteEvidenceEventRow {
    pub async fn list_by_quote_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> DbResult<Vec<QuoteEvidenceEventRow>> {
        use crate::schema::quote_evidence_event::dsl as d;

        let query = d::quote_evidence_event
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .order(d::seq.asc())
            .select(QuoteEvidenceEventRow::as_select());
        crate::debug_sql(&query);
        query
            .get_results(conn)
            .await
            .attach("Error while listing quote evidence")
            .into_db_result()
    }

    pub async fn last_by_quote_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> DbResult<Option<QuoteEvidenceEventRow>> {
        use crate::schema::quote_evidence_event::dsl as d;

        let query = d::quote_evidence_event
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .order(d::seq.desc())
            .select(QuoteEvidenceEventRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while reading last quote evidence")
            .into_db_result()
    }
}

impl QuoteSignatureOtpRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<QuoteSignatureOtpRow> {
        use crate::schema::quote_signature_otp::dsl as d;

        let query = diesel::insert_into(d::quote_signature_otp).values(self);
        crate::debug_sql(&query);
        query
            .returning(QuoteSignatureOtpRow::as_returning())
            .get_result(conn)
            .await
            .attach("Error while inserting quote signature code")
            .into_db_result()
    }
}

impl QuoteSignatureOtpRow {
    pub async fn latest_for(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
        email: &str,
    ) -> DbResult<Option<QuoteSignatureOtpRow>> {
        use crate::schema::quote_signature_otp::dsl as d;

        let query = d::quote_signature_otp
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .filter(d::email.eq(email))
            .order(d::created_at.desc())
            .select(QuoteSignatureOtpRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while reading quote signature code")
            .into_db_result()
    }

    pub async fn count_since(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
        email: &str,
        since: chrono::NaiveDateTime,
    ) -> DbResult<i64> {
        use crate::schema::quote_signature_otp::dsl as d;

        let query = d::quote_signature_otp
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .filter(d::email.eq(email))
            .filter(d::created_at.gt(since))
            .count();
        crate::debug_sql(&query);
        query
            .get_result(conn)
            .await
            .attach("Error while counting quote signature codes")
            .into_db_result()
    }

    pub async fn record_attempt(
        conn: &mut PgConn,
        id: QuoteSignatureOtpId,
        verified: bool,
    ) -> DbResult<()> {
        use crate::schema::quote_signature_otp::dsl as d;

        let now = common_clock::now().naive_utc();
        let query = diesel::update(d::quote_signature_otp)
            .filter(d::id.eq(id))
            .set((
                d::attempts.eq(d::attempts + 1),
                d::verified_at.eq(verified.then_some(now)),
            ));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while recording a quote signature code attempt")
            .into_db_result()
    }
}

impl QuoteSignatureOtpRow {
    pub async fn consume(conn: &mut PgConn, id: QuoteSignatureOtpId) -> DbResult<()> {
        use crate::schema::quote_signature_otp::dsl as d;

        let query = diesel::update(d::quote_signature_otp)
            .filter(d::id.eq(id))
            .set(d::consumed_at.eq(common_clock::now().naive_utc()));
        crate::debug_sql(&query);
        query
            .execute(conn)
            .await
            .map(|_| ())
            .attach("Error while consuming a quote signature code")
            .into_db_result()
    }
}

impl QuoteSignatureCertificateRow {
    /// `None` when the quote already has one: the first certificate issued stays.
    pub async fn insert_if_absent(
        &self,
        conn: &mut PgConn,
    ) -> DbResult<Option<QuoteSignatureCertificateRow>> {
        use crate::schema::quote_signature_certificate::dsl as d;

        let query = diesel::insert_into(d::quote_signature_certificate)
            .values(self)
            .on_conflict(d::quote_id)
            .do_nothing();
        crate::debug_sql(&query);
        query
            .returning(QuoteSignatureCertificateRow::as_returning())
            .get_result(conn)
            .await
            .optional()
            .attach("Error while storing quote signature certificate")
            .into_db_result()
    }

    pub async fn find(
        conn: &mut PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> DbResult<Option<QuoteSignatureCertificateRow>> {
        use crate::schema::quote_signature_certificate::dsl as d;

        let query = d::quote_signature_certificate
            .filter(d::tenant_id.eq(tenant_id))
            .filter(d::quote_id.eq(quote_id))
            .select(QuoteSignatureCertificateRow::as_select());
        crate::debug_sql(&query);
        query
            .first(conn)
            .await
            .optional()
            .attach("Error while reading quote signature certificate")
            .into_db_result()
    }
}
