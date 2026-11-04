use crate::entity_activity::EntityActivityRow;
use crate::errors::IntoDbResult;
use crate::sent_email::{SentEmailRow, SentEmailRowNew, SentEmailWithEntityRow};
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use common_domain::ids::{EntityActivityId, InvoicingEntityId, TenantId};
use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl, SelectableHelper};
use error_stack::ResultExt;
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct SentEmailFilter {
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    /// `EmailKind` snake_case value.
    pub kind: Option<String>,
    pub entity_type: Option<String>,
    pub entity_id: Option<Uuid>,
    /// Emails sent to the customer or about one of its entities.
    pub customer_id: Option<Uuid>,
}

/// Keyset cursor on `(sent_at DESC, id DESC)`.
#[derive(Debug, Clone, Copy)]
pub struct SentEmailCursor {
    pub sent_at: NaiveDateTime,
    pub id: EntityActivityId,
}

/// Activity types recording a customer email that was not sent.
pub const EMAIL_SKIPPED: &str = "entity.email_skipped";
pub const PAYMENT_REMINDER_SKIPPED: &str = "payment_reminder.skipped";

#[derive(Debug, Clone, Default)]
pub struct SkippedEmailFilter {
    pub invoicing_entity_id: Option<InvoicingEntityId>,
    /// `EmailKind` snake_case value.
    pub kind: Option<String>,
}

/// Same `(time DESC, id DESC)` keyset; `sent_at` holds the skip's `occurred_at`.
pub type SkippedEmailCursor = SentEmailCursor;

impl SentEmailRowNew {
    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<SentEmailRow> {
        use crate::schema::sent_email::dsl::sent_email;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(sent_email).values(self);
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while inserting sent_email")
            .into_db_result()
    }
}

impl SentEmailRow {
    /// Whether an email of `kind` about exactly `context` was recorded at or after `since`.
    pub async fn exists_for_context_since(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_kind: &str,
        param_context: serde_json::Value,
        since: NaiveDateTime,
    ) -> DbResult<bool> {
        use crate::schema::sent_email::dsl as se;
        use diesel_async::RunQueryDsl;

        diesel::select(diesel::dsl::exists(
            se::sent_email
                .filter(se::tenant_id.eq(param_tenant_id))
                .filter(se::kind.eq(param_kind))
                .filter(se::context.eq(param_context))
                .filter(se::sent_at.ge(since)),
        ))
        .get_result::<bool>(conn)
        .await
        .attach("Error while looking for a later sent_email")
        .into_db_result()
    }

    pub async fn find_by_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_id: EntityActivityId,
    ) -> DbResult<SentEmailRow> {
        use crate::schema::sent_email::dsl as se;
        use diesel_async::RunQueryDsl;

        se::sent_email
            .filter(se::id.eq(param_id))
            .filter(se::tenant_id.eq(param_tenant_id))
            .select(SentEmailRow::as_select())
            .get_result(conn)
            .await
            .attach("Error while finding sent_email by id")
            .into_db_result()
    }

    pub async fn find_with_entity_by_id(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        param_id: EntityActivityId,
    ) -> DbResult<Option<SentEmailWithEntityRow>> {
        use crate::schema::entity_activity::dsl as ea;
        use crate::schema::sent_email::dsl as se;
        use diesel::OptionalExtension;
        use diesel_async::RunQueryDsl;

        let query = se::sent_email
            .inner_join(ea::entity_activity)
            .filter(se::tenant_id.eq(param_tenant_id))
            .filter(se::id.eq(param_id))
            .select(SentEmailWithEntityRow::as_select());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding sent_email with its activity")
            .into_db_result()
    }

    pub async fn list(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        filter: &SentEmailFilter,
        before: Option<SentEmailCursor>,
        limit: i64,
    ) -> DbResult<Vec<SentEmailWithEntityRow>> {
        use crate::schema::entity_activity::dsl as ea;
        use crate::schema::sent_email::dsl as se;
        use diesel_async::RunQueryDsl;

        let mut query = se::sent_email
            .inner_join(ea::entity_activity)
            .filter(se::tenant_id.eq(param_tenant_id))
            .order((se::sent_at.desc(), se::id.desc()))
            .select(SentEmailWithEntityRow::as_select())
            .limit(limit)
            .into_boxed();

        if let Some(entity) = filter.invoicing_entity_id {
            query = query.filter(se::invoicing_entity_id.eq(entity));
        }
        if let Some(ref kind) = filter.kind {
            query = query.filter(se::kind.eq(kind));
        }
        if let Some(ref entity_type) = filter.entity_type {
            query = query.filter(ea::entity_type.eq(entity_type));
        }
        if let Some(entity_id) = filter.entity_id {
            query = query.filter(ea::entity_id.eq(entity_id));
        }
        if let Some(customer) = filter.customer_id {
            query = query.filter(
                ea::agg_customer_id.eq(customer).or(ea::entity_type
                    .eq("customer")
                    .and(ea::entity_id.eq(customer))),
            );
        }
        if let Some(cur) = before {
            query = query.filter(
                se::sent_at
                    .lt(cur.sent_at)
                    .or(se::sent_at.eq(cur.sent_at).and(se::id.lt(cur.id))),
            );
        }
        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing sent_email")
            .into_db_result()
    }
}

impl EntityActivityRow {
    /// Skip entries of customer emails, newest first.
    pub async fn list_skipped_emails(
        conn: &mut PgConn,
        param_tenant_id: TenantId,
        filter: &SkippedEmailFilter,
        before: Option<SkippedEmailCursor>,
        limit: i64,
    ) -> DbResult<Vec<EntityActivityRow>> {
        use crate::schema::credit_note::dsl as cn;
        use crate::schema::customer::dsl as c;
        use crate::schema::entity_activity::dsl as ea;
        use crate::schema::invoice::dsl as i;
        use diesel::dsl::{exists, sql};
        use diesel::sql_types::Bool;
        use diesel::{NullableExpressionMethods, PgAnyJsonExpressionMethods};
        use diesel_async::RunQueryDsl;

        // Inlined, not bound, so the planner can prove `idx_entity_activity_email_skipped`.
        let skip_types = sql::<Bool>(&format!(
            "entity_activity.activity_type IN ('{EMAIL_SKIPPED}', '{PAYMENT_REMINDER_SKIPPED}')"
        ));
        let mut query = ea::entity_activity
            .filter(ea::tenant_id.eq(param_tenant_id))
            .filter(skip_types)
            .order((ea::occurred_at.desc(), ea::id.desc()))
            .select(EntityActivityRow::as_select())
            .limit(limit)
            .into_boxed();

        if let Some(entity) = filter.invoicing_entity_id {
            let on_invoice = exists(
                i::invoice
                    .filter(i::id.eq(ea::entity_id))
                    .filter(i::tenant_id.eq(param_tenant_id))
                    .filter(i::invoicing_entity_id.eq(entity))
                    .select(i::id),
            );
            let on_credit_note = exists(
                cn::credit_note
                    .filter(cn::id.eq(ea::entity_id))
                    .filter(cn::tenant_id.eq(param_tenant_id))
                    .filter(cn::invoicing_entity_id.eq(entity))
                    .select(cn::id),
            );
            // A quote has no entity of its own: it goes out under its customer's.
            let on_quote = exists(
                c::customer
                    .filter(c::id.nullable().eq(ea::agg_customer_id))
                    .filter(c::tenant_id.eq(param_tenant_id))
                    .filter(c::invoicing_entity_id.eq(entity))
                    .select(c::id),
            );
            query = query.filter(
                ea::entity_type
                    .eq("invoice")
                    .and(on_invoice)
                    .or(ea::entity_type.eq("credit_note").and(on_credit_note))
                    .or(ea::entity_type.eq("quote").and(on_quote)),
            );
        }
        if let Some(ref kind) = filter.kind {
            // Reminder skips are typed by activity, whatever their metadata holds.
            query = if kind == "payment_reminder" {
                query.filter(ea::activity_type.eq(PAYMENT_REMINDER_SKIPPED))
            } else {
                query
                    .filter(ea::activity_type.eq(EMAIL_SKIPPED))
                    .filter(ea::metadata.retrieve_as_text("kind").eq(kind))
            };
        }
        if let Some(cur) = before {
            query = query.filter(
                ea::occurred_at
                    .lt(cur.sent_at)
                    .or(ea::occurred_at.eq(cur.sent_at).and(ea::id.lt(cur.id))),
            );
        }
        crate::debug_sql(&query);

        query
            .load(conn)
            .await
            .attach("Error while listing skipped emails")
            .into_db_result()
    }
}
