use crate::errors::IntoDbResult;
use crate::invoicing_entities::{
    InvoicingEntityProvidersRow, InvoicingEntityRow, InvoicingEntityRowPatch,
    InvoicingEntityRowProvidersPatch,
};

use crate::{DbResult, PgConn};

use crate::bank_accounts::BankAccountRow;
use crate::connectors::ConnectorRow;
use common_domain::ids::{InvoiceId, InvoicingEntityId, TenantId};
use diesel::{
    ExpressionMethods, JoinOnDsl, NullableExpressionMethods, QueryDsl, Selectable,
    SelectableHelper, alias, debug_query,
};
use error_stack::ResultExt;

impl InvoicingEntityRow {
    pub async fn list_by_ids(
        conn: &mut PgConn,
        ids: Vec<InvoicingEntityId>,
    ) -> DbResult<Vec<InvoicingEntityRow>> {
        use crate::schema::invoicing_entity::dsl;
        use diesel_async::RunQueryDsl;

        let query = dsl::invoicing_entity
            .filter(dsl::id.eq_any(ids))
            .select(InvoicingEntityRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching invoicing entities by ids")
            .into_db_result()
    }

    pub async fn insert(&self, conn: &mut PgConn) -> DbResult<InvoicingEntityRow> {
        use crate::schema::invoicing_entity::dsl::*;
        use diesel_async::RunQueryDsl;

        let query = diesel::insert_into(invoicing_entity).values(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while inserting organization")
            .into_db_result()
    }
    pub async fn list_by_tenant_id(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<Vec<InvoicingEntityRow>> {
        use crate::schema::invoicing_entity::dsl;
        use diesel_async::RunQueryDsl;

        let query = dsl::invoicing_entity
            .filter(dsl::tenant_id.eq(tenant_id))
            .select(InvoicingEntityRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching invoicing entities by tenant")
            .into_db_result()
    }

    pub async fn exists_any_for_tenant(conn: &mut PgConn, tenant_id: TenantId) -> DbResult<bool> {
        use crate::schema::invoicing_entity::dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::dsl::select(diesel::dsl::exists(
            dsl::invoicing_entity.filter(dsl::tenant_id.eq(tenant_id)),
        ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while checking if tenant has any invoicing entities")
            .into_db_result()
    }

    pub async fn is_in_use(
        conn: &mut PgConn,
        invoicing_entity_id: InvoicingEntityId,
        tenant_id: TenantId,
    ) -> DbResult<bool> {
        use crate::schema::customer::dsl as c_dsl;
        use crate::schema::invoice::dsl as i_dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::dsl::select(diesel::dsl::exists(
            i_dsl::invoice
                .inner_join(c_dsl::customer.on(i_dsl::customer_id.eq(c_dsl::id)))
                .filter(i_dsl::tenant_id.eq(tenant_id))
                .filter(c_dsl::invoicing_entity_id.eq(invoicing_entity_id)),
        ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while checking if tenant has any invoicing entities")
            .into_db_result()
    }

    pub async fn get_default_invoicing_entity_for_tenant(
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<InvoicingEntityRow> {
        use crate::schema::invoicing_entity::dsl;
        use diesel_async::RunQueryDsl;

        let query = dsl::invoicing_entity
            .filter(dsl::tenant_id.eq(tenant_id))
            .filter(dsl::is_default.eq(true))
            .select(InvoicingEntityRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while fetching default invoicing entity by tenant")
            .into_db_result()
    }

    pub async fn get_invoicing_entity_by_id_and_tenant(
        conn: &mut PgConn,
        id: InvoicingEntityId,
        tenant_id: TenantId,
    ) -> DbResult<InvoicingEntityRow> {
        use crate::schema::invoicing_entity::dsl;
        use diesel_async::RunQueryDsl;

        let query = dsl::invoicing_entity
            .filter(dsl::id.eq(id))
            .filter(dsl::tenant_id.eq(tenant_id))
            .select(InvoicingEntityRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while fetching invoicing entity by id and tenant")
            .into_db_result()
    }

    pub async fn select_for_update_by_id_and_tenant(
        conn: &mut PgConn,
        id: InvoicingEntityId,
        tenant_id: TenantId,
    ) -> DbResult<InvoicingEntityRow> {
        use crate::schema::invoicing_entity::dsl;
        use diesel_async::RunQueryDsl;

        let query = dsl::invoicing_entity
            .for_no_key_update()
            .filter(dsl::id.eq(id))
            .filter(dsl::tenant_id.eq(tenant_id))
            .select(InvoicingEntityRow::as_select());

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while fetching invoicing entity by id and tenant")
            .into_db_result()
    }

    pub async fn update_invoicing_entity_number(
        conn: &mut PgConn,
        id: InvoicingEntityId,
        tenant_id: TenantId,
        new_invoice_number: i64,
    ) -> DbResult<InvoicingEntityRow> {
        use crate::schema::invoicing_entity::dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(dsl::invoicing_entity)
            .filter(dsl::id.eq(id))
            .filter(dsl::tenant_id.eq(tenant_id))
            .set(dsl::next_invoice_number.eq(new_invoice_number + 1));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while updating invoicing entity number")
            .into_db_result()
    }
}

impl InvoicingEntityProvidersRow {
    pub async fn resolve_providers_by_id(
        conn: &mut PgConn,
        id: InvoicingEntityId,
        tenant_id: TenantId,
    ) -> DbResult<InvoicingEntityProvidersRow> {
        use crate::schema::bank_account::dsl as b_dsl;
        use crate::schema::connector::dsl as c_dsl;
        use crate::schema::invoicing_entity::dsl as i_dsl;

        use diesel::alias;
        use diesel_async::RunQueryDsl;

        let (card_connector, direct_debut_connector) = alias!(
            crate::schema::connector as card_connector,
            crate::schema::connector as direct_debut_connector
        );

        let query = i_dsl::invoicing_entity
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .filter(i_dsl::id.eq(id))
            .left_join(b_dsl::bank_account.on(i_dsl::bank_account_id.eq(b_dsl::id.nullable())))
            .left_join(
                card_connector
                    .on(i_dsl::card_provider_id.eq(card_connector.field(c_dsl::id).nullable())),
            )
            .left_join(
                direct_debut_connector.on(i_dsl::direct_debit_provider_id
                    .eq(direct_debut_connector.field(c_dsl::id).nullable())),
            )
            .select((
                InvoicingEntityRow::as_select(),
                card_connector
                    .fields(<ConnectorRow as Selectable<diesel::pg::Pg>>::construct_selection())
                    .nullable(),
                direct_debut_connector
                    .fields(<ConnectorRow as Selectable<diesel::pg::Pg>>::construct_selection())
                    .nullable(),
                Option::<BankAccountRow>::as_select(),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .first(conn)
            .await
            .attach_printable("Error while fetching default invoicing entity by tenant")
            .into_db_result()
    }

    pub async fn list_by_tenant_id(
        conn: &mut PgConn,
        tenant_id: &TenantId,
    ) -> DbResult<Vec<InvoicingEntityProvidersRow>> {
        use crate::schema::bank_account::dsl as b_dsl;
        use crate::schema::connector::dsl as c_dsl;
        use crate::schema::invoicing_entity::dsl as i_dsl;

        use diesel::alias;
        use diesel_async::RunQueryDsl;

        let (card_connector, direct_debut_connector) = alias!(
            crate::schema::connector as card_connector,
            crate::schema::connector as direct_debut_connector
        );

        let query = i_dsl::invoicing_entity
            .filter(i_dsl::tenant_id.eq(tenant_id))
            .left_join(b_dsl::bank_account.on(i_dsl::bank_account_id.eq(b_dsl::id.nullable())))
            .left_join(
                card_connector
                    .on(i_dsl::card_provider_id.eq(card_connector.field(c_dsl::id).nullable())),
            )
            .left_join(
                direct_debut_connector.on(i_dsl::direct_debit_provider_id
                    .eq(direct_debut_connector.field(c_dsl::id).nullable())),
            )
            .select((
                InvoicingEntityRow::as_select(),
                card_connector
                    .fields(<ConnectorRow as Selectable<diesel::pg::Pg>>::construct_selection())
                    .nullable(),
                direct_debut_connector
                    .fields(<ConnectorRow as Selectable<diesel::pg::Pg>>::construct_selection())
                    .nullable(),
                Option::<BankAccountRow>::as_select(),
            ));

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_results(conn)
            .await
            .attach_printable("Error while fetching invoicing entities by tenant")
            .into_db_result()
    }
}

impl InvoicingEntityRowPatch {
    pub async fn patch_invoicing_entity(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<InvoicingEntityRow> {
        use crate::schema::invoicing_entity::dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(dsl::invoicing_entity)
            .filter(dsl::id.eq(self.id))
            .filter(dsl::tenant_id.eq(tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while patching invoicing entity")
            .into_db_result()
    }
}

impl InvoicingEntityRowProvidersPatch {
    pub async fn patch_invoicing_entity_providers(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
    ) -> DbResult<InvoicingEntityRow> {
        use crate::schema::invoicing_entity::dsl;
        use diesel_async::RunQueryDsl;

        let query = diesel::update(dsl::invoicing_entity)
            .filter(dsl::id.eq(self.id))
            .filter(dsl::tenant_id.eq(tenant_id))
            .set(self);

        log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

        query
            .get_result(conn)
            .await
            .attach_printable("Error while patching invoicing entity")
            .into_db_result()
    }
}

pub async fn get_invoicing_entity_id_by_invoice_id(
    conn: &mut PgConn,
    tenant_id: TenantId,
    invoice_id: InvoiceId,
) -> DbResult<InvoicingEntityId> {
    use crate::schema::customer::dsl as c_dsl;
    use crate::schema::invoice::dsl as i_dsl;
    use diesel_async::RunQueryDsl;

    let query = i_dsl::invoice
        .inner_join(c_dsl::customer.on(i_dsl::customer_id.eq(c_dsl::id)))
        .filter(i_dsl::tenant_id.eq(tenant_id))
        .filter(i_dsl::id.eq(invoice_id))
        .select(c_dsl::invoicing_entity_id);

    log::debug!("{}", debug_query::<diesel::pg::Pg, _>(&query).to_string());

    query
        .get_result(conn)
        .await
        .attach_printable("Error while retrieving invoicing entity id by invoice id")
        .into_db_result()
}
