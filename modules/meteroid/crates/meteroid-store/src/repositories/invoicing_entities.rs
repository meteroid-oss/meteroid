use chrono::{Datelike, NaiveDate};
use error_stack::Report;
use uuid::Uuid;

use diesel_models::invoicing_entities::{InvoicingEntityRow, InvoicingEntityRowPatch};
use diesel_models::organizations::OrganizationRow;

use crate::domain::invoicing_entities::InvoicingEntity;
use crate::domain::{InvoicingEntityNew, InvoicingEntityPatch};
use crate::errors::StoreError;
use crate::store::{PgConn, Store, StoreInternal};
use crate::utils::local_id::{IdType, LocalId};
use crate::StoreResult;

#[async_trait::async_trait]
pub trait InvoicingEntityInterface {
    async fn list_invoicing_entities(&self, tenant_id: Uuid) -> StoreResult<Vec<InvoicingEntity>>;

    async fn list_invoicing_entities_by_ids(
        &self,
        ids: Vec<Uuid>,
    ) -> StoreResult<Vec<InvoicingEntity>>;

    async fn get_invoicing_entity(
        &self,
        tenant_id: Uuid,
        invoicing_id_or_default: Option<Uuid>,
    ) -> StoreResult<InvoicingEntity>;

    async fn create_invoicing_entity(
        &self,
        invoicing_entity: InvoicingEntityNew,
        tenant_id: Uuid,
        organization_id: Uuid,
    ) -> StoreResult<InvoicingEntity>;
    async fn patch_invoicing_entity(
        &self,
        invoicing_entity: InvoicingEntityPatch,
        tenant_id: Uuid,
    ) -> StoreResult<InvoicingEntity>;
}

#[async_trait::async_trait]
impl InvoicingEntityInterface for Store {
    async fn list_invoicing_entities(&self, tenant_id: Uuid) -> StoreResult<Vec<InvoicingEntity>> {
        let mut conn = self.get_conn().await?;

        let invoicing_entities = InvoicingEntityRow::list_by_tenant_id(&mut conn, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|v| v.into())
            .collect();

        Ok(invoicing_entities)
    }

    async fn list_invoicing_entities_by_ids(
        &self,
        ids: Vec<Uuid>,
    ) -> StoreResult<Vec<InvoicingEntity>> {
        let mut conn = self.get_conn().await?;

        let invoicing_entities = InvoicingEntityRow::list_by_ids(&mut conn, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|v| v.into())
            .collect();

        Ok(invoicing_entities)
    }

    async fn get_invoicing_entity(
        &self,
        tenant_id: Uuid,
        invoicing_id_or_default: Option<Uuid>,
    ) -> StoreResult<InvoicingEntity> {
        let mut conn = self.get_conn().await?;

        let invoicing_entity = match invoicing_id_or_default {
            Some(invoicing_id) => InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                &mut conn,
                &invoicing_id,
                &tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into(),
            None => {
                InvoicingEntityRow::get_default_invoicing_entity_for_tenant(&mut conn, &tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
                    .into()
            }
        };

        Ok(invoicing_entity)
    }

    async fn create_invoicing_entity(
        &self,
        invoicing_entity: InvoicingEntityNew,
        tenant_id: Uuid,
        organization_id: Uuid,
    ) -> StoreResult<InvoicingEntity> {
        let mut conn = self.get_conn().await?;

        let organization = OrganizationRow::get_by_id(&mut conn, organization_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        self.internal
            .create_invoicing_entity(
                &mut conn,
                invoicing_entity,
                tenant_id,
                organization.default_country,
                organization.trade_name,
            )
            .await
    }

    async fn patch_invoicing_entity(
        &self,
        invoicing_entity: InvoicingEntityPatch,
        tenant_id: Uuid,
    ) -> StoreResult<InvoicingEntity> {
        let mut conn = self.get_conn().await?;

        let mut row: InvoicingEntityRowPatch = invoicing_entity.into();

        if row.country.is_some() {
            let is_in_use = InvoicingEntityRow::is_in_use(&mut conn, &row.id, &tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            // we don't allow country changes if already in use
            if is_in_use {
                row.country = None;
            } else {
                let currency = self
                    .internal
                    .get_currency_from_country(&row.country.clone().unwrap())?;
                row.accounting_currency = Some(currency);
            }
        }

        let res = row
            .patch_invoicing_entity(&mut conn, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(res.into())
    }
}

impl StoreInternal {
    pub async fn create_invoicing_entity(
        &self,
        conn: &mut PgConn,
        invoicing_entity: InvoicingEntityNew,
        tenant_id: Uuid,
        default_country: String,
        trade_name: String,
    ) -> StoreResult<InvoicingEntity> {
        let other_exists = InvoicingEntityRow::exists_any_for_tenant(conn, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let country = invoicing_entity
            .country
            .clone()
            .unwrap_or(default_country.clone());

        let currency = self.get_currency_from_country(&country)?;

        let entity = InvoicingEntity {
            id: Uuid::new_v4(),
            local_id: LocalId::generate_for(IdType::InvoicingEntity),
            is_default: !other_exists,
            legal_name: invoicing_entity.legal_name.unwrap_or(trade_name),
            invoice_number_pattern: invoicing_entity
                .invoice_number_pattern
                .unwrap_or("INV-{number}".to_string()),
            next_invoice_number: 1,
            next_credit_note_number: 1,
            grace_period_hours: invoicing_entity.grace_period_hours.unwrap_or(24),
            net_terms: invoicing_entity.net_terms.unwrap_or(30),
            invoice_footer_info: invoicing_entity.invoice_footer_info.clone(),
            invoice_footer_legal: invoicing_entity.invoice_footer_legal.clone(),
            logo_attachment_id: invoicing_entity.logo_attachment_id.clone(),
            brand_color: invoicing_entity.brand_color.clone(),
            address_line1: invoicing_entity.address_line1.clone(),
            address_line2: invoicing_entity.address_line2.clone(),
            zip_code: invoicing_entity.zip_code.clone(),
            state: invoicing_entity.state.clone(),
            city: invoicing_entity.city.clone(),
            vat_number: invoicing_entity.vat_number.clone(),
            country: country,
            accounting_currency: currency,
            tenant_id,
        };

        let row: InvoicingEntityRow = entity.into();

        let invoicing_entity_row = row
            .insert(conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(invoicing_entity_row.into())
    }

    pub fn format_invoice_number(&self, number: i64, format: String, date: NaiveDate) -> String {
        format
            .replace("{number}", &number.to_string())
            .replace("{YYYY}", &date.year().to_string())
            .replace("{MM}", &date.month().to_string())
            .replace("{DD}", &date.day().to_string())
    }
}
