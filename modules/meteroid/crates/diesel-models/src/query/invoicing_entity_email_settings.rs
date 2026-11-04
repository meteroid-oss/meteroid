use crate::errors::IntoDbResult;
use crate::invoicing_entity_email_settings::{
    InvoicingEntityEmailSettingsRow, InvoicingEntityEmailSettingsRowNew,
};
use crate::{DbResult, PgConn};
use chrono::NaiveDateTime;
use common_domain::ids::{InvoicingEntityId, TenantId};
use diesel::upsert::excluded;
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl, SelectableHelper};
use diesel_async::RunQueryDsl;
use error_stack::ResultExt;

diesel::define_sql_function! {
    fn array_remove(
        arr: diesel::sql_types::Array<diesel::sql_types::Nullable<diesel::sql_types::Text>>,
        elem: diesel::sql_types::Text
    ) -> diesel::sql_types::Array<diesel::sql_types::Nullable<diesel::sql_types::Text>>;
}
diesel::define_sql_function! {
    fn array_append(
        arr: diesel::sql_types::Array<diesel::sql_types::Nullable<diesel::sql_types::Text>>,
        elem: diesel::sql_types::Text
    ) -> diesel::sql_types::Array<diesel::sql_types::Nullable<diesel::sql_types::Text>>;
}

impl InvoicingEntityEmailSettingsRowNew {
    /// Lazy creation next to the entity; an existing row is left untouched.
    pub async fn insert_if_absent(
        &self,
        conn: &mut PgConn,
    ) -> DbResult<Option<InvoicingEntityEmailSettingsRow>> {
        use crate::schema::invoicing_entity_email_settings::dsl as s_dsl;

        let query = diesel::insert_into(s_dsl::invoicing_entity_email_settings)
            .values(self)
            .on_conflict_do_nothing()
            .returning(InvoicingEntityEmailSettingsRow::as_returning());
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .optional()
            .attach("Error while inserting invoicing_entity_email_settings")
            .into_db_result()
    }

    /// Settings save: replaces the sender fields, keeps the stored `disabled_kinds`
    /// (`self.disabled_kinds` only seeds a row that does not exist yet).
    pub async fn upsert_settings(
        &self,
        conn: &mut PgConn,
        now: NaiveDateTime,
    ) -> DbResult<InvoicingEntityEmailSettingsRow> {
        use crate::schema::invoicing_entity_email_settings::dsl as s_dsl;

        let query = diesel::insert_into(s_dsl::invoicing_entity_email_settings)
            .values(self)
            .on_conflict(s_dsl::invoicing_entity_id)
            .do_update()
            .set((
                s_dsl::sender_name.eq(excluded(s_dsl::sender_name)),
                s_dsl::reply_to.eq(excluded(s_dsl::reply_to)),
                s_dsl::bcc.eq(excluded(s_dsl::bcc)),
                s_dsl::timezone.eq(excluded(s_dsl::timezone)),
                s_dsl::reminder_send_hour.eq(excluded(s_dsl::reminder_send_hour)),
                s_dsl::updated_at.eq(now),
            ))
            .returning(InvoicingEntityEmailSettingsRow::as_returning());
        crate::debug_sql(&query);

        query
            .get_result(conn)
            .await
            .attach("Error while upserting invoicing_entity_email_settings")
            .into_db_result()
    }

    /// Toggles one kind: an existing row gets `kind` removed from / appended to its
    /// `disabled_kinds`; a missing row is created from `self`, whose set must already
    /// reflect the toggle. The only path that removes a kind from the set.
    pub async fn set_kind_enabled(
        &self,
        conn: &mut PgConn,
        kind: &str,
        enabled: bool,
        now: NaiveDateTime,
    ) -> DbResult<InvoicingEntityEmailSettingsRow> {
        use crate::schema::invoicing_entity_email_settings::dsl as s_dsl;

        let insert = diesel::insert_into(s_dsl::invoicing_entity_email_settings)
            .values(self)
            .on_conflict(s_dsl::invoicing_entity_id)
            .do_update();
        let result = if enabled {
            let query = insert
                .set((
                    s_dsl::disabled_kinds.eq(array_remove(s_dsl::disabled_kinds, kind)),
                    s_dsl::updated_at.eq(now),
                ))
                .returning(InvoicingEntityEmailSettingsRow::as_returning());
            crate::debug_sql(&query);
            query.get_result(conn).await
        } else {
            let query = insert
                .set((
                    s_dsl::disabled_kinds.eq(array_append(
                        array_remove(s_dsl::disabled_kinds, kind),
                        kind,
                    )),
                    s_dsl::updated_at.eq(now),
                ))
                .returning(InvoicingEntityEmailSettingsRow::as_returning());
            crate::debug_sql(&query);
            query.get_result(conn).await
        };

        result
            .attach("Error while toggling an email kind")
            .into_db_result()
    }
}

impl InvoicingEntityEmailSettingsRow {
    pub async fn find_by_entity_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
        invoicing_entity_id: InvoicingEntityId,
    ) -> DbResult<Option<InvoicingEntityEmailSettingsRow>> {
        use crate::schema::invoicing_entity_email_settings::dsl as s_dsl;

        let query = s_dsl::invoicing_entity_email_settings
            .filter(s_dsl::tenant_id.eq(tenant_id))
            .filter(s_dsl::invoicing_entity_id.eq(invoicing_entity_id))
            .select(InvoicingEntityEmailSettingsRow::as_select());
        crate::debug_sql(&query);

        query
            .first(conn)
            .await
            .optional()
            .attach("Error while finding invoicing_entity_email_settings")
            .into_db_result()
    }
}
