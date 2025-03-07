use crate::domain::add_ons::AddOn;
use crate::domain::coupons::Coupon;
use crate::domain::{
    CreateSubscription, Customer, CustomerConnection, InvoicingEntityProviderSensitive,
    PlanForSubscription, PriceComponent,
};
use crate::errors::StoreError;
use crate::store::{PgConn, StoreInternal};
use crate::StoreResult;
use common_domain::ids::TenantId;
use diesel_models::add_ons::AddOnRow;
use diesel_models::coupons::CouponRow;
use diesel_models::customer_connection::CustomerConnectionRow;
use diesel_models::customers::CustomerRow;
use diesel_models::invoicing_entities::InvoicingEntityProvidersRow;
use diesel_models::plans::PlanRowForSubscription;
use diesel_models::price_components::PriceComponentRow;
use error_stack::Report;
use itertools::Itertools;
use secrecy::SecretString;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug)]
pub struct SubscriptionCreationContext {
    pub customers: Vec<Customer>,
    pub customer_connection: Vec<CustomerConnection>,
    pub plans: Vec<PlanForSubscription>,
    pub price_components_by_plan_version: HashMap<Uuid, Vec<PriceComponent>>,
    pub all_add_ons: Vec<AddOn>,
    pub all_coupons: Vec<Coupon>,
    pub invoicing_entity_providers: Vec<InvoicingEntityProviderSensitive>,
}

impl SubscriptionCreationContext {
    pub(crate) fn get_invoicing_entity_providers_for_customer(
        &self,
        customer: &Customer,
    ) -> Option<&InvoicingEntityProviderSensitive> {
        self.invoicing_entity_providers
            .iter()
            .find(|e| e.id == customer.invoicing_entity_id)
    }

    pub(crate) fn get_customer_connection_for_customer(
        &self,
        customer: &Customer,
    ) -> Vec<&CustomerConnection> {
        self.customer_connection
            .iter()
            .filter(|e| e.customer_id == customer.id)
            .collect()
    }
}

impl StoreInternal {
    pub(super) async fn gather_subscription_context(
        &self,
        conn: &mut PgConn,
        batch: &[CreateSubscription],
        tenant_id: TenantId,
        secret_decoding_key: &SecretString,
    ) -> StoreResult<SubscriptionCreationContext> {
        let plan_version_ids: Vec<_> = batch
            .iter()
            .map(|c| c.subscription.plan_version_id)
            .collect();

        // hard to parallelize within a tx (mutable), we could use a single query TODO(PERF)
        let plans = self.get_plans(conn, &plan_version_ids).await?;
        let price_components = self.get_price_components(conn, &plan_version_ids).await?;
        let add_ons = self.get_add_ons(conn, batch, &tenant_id).await?;
        let coupons = self.get_coupons(conn, batch, &tenant_id).await?;
        let customers = self.get_customers(conn, batch, &tenant_id).await?;
        let customer_connection = self
            .get_customer_connection(conn, batch, &tenant_id)
            .await?;
        let invoicing_entities = self
            .list_invoicing_entities(conn, &tenant_id, secret_decoding_key)
            .await?;

        Ok(SubscriptionCreationContext {
            customers,
            plans,
            price_components_by_plan_version: price_components,
            all_add_ons: add_ons,
            all_coupons: coupons,
            invoicing_entity_providers: invoicing_entities,
            customer_connection,
        })
    }

    async fn list_invoicing_entities(
        &self,
        conn: &mut PgConn,
        tenant_id: &TenantId,
        secret_decoding_key: &SecretString,
    ) -> StoreResult<Vec<InvoicingEntityProviderSensitive>> {
        InvoicingEntityProvidersRow::list_by_tenant_id(conn, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|s| InvoicingEntityProviderSensitive::from_row(s, secret_decoding_key))
            .collect::<Result<Vec<_>, _>>()
    }
    async fn get_plans(
        &self,
        conn: &mut PgConn,
        plan_version_ids: &[Uuid],
    ) -> StoreResult<Vec<PlanForSubscription>> {
        PlanRowForSubscription::get_plans_for_subscription_by_version_ids(
            conn,
            plan_version_ids.to_vec(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .map(|x| x.into_iter().map(Into::into).collect())
    }

    async fn get_price_components(
        &self,
        conn: &mut PgConn,
        plan_version_ids: &[Uuid],
    ) -> StoreResult<HashMap<Uuid, Vec<PriceComponent>>> {
        PriceComponentRow::get_by_plan_ids(conn, plan_version_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|(k, v)| {
                let converted_vec: error_stack::Result<Vec<PriceComponent>, _> =
                    v.into_iter().map(TryInto::try_into).collect();
                converted_vec.map(|vec| (k, vec))
            })
            .collect()
    }

    async fn get_add_ons(
        &self,
        conn: &mut PgConn,
        batch: &[CreateSubscription],
        tenant_id: &TenantId,
    ) -> StoreResult<Vec<AddOn>> {
        let add_on_ids: Vec<_> = batch
            .iter()
            .filter_map(|x| x.add_ons.as_ref())
            .flat_map(|x| &x.add_ons)
            .map(|x| x.add_on_id)
            .unique()
            .collect();

        AddOnRow::list_by_ids(conn, &add_on_ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(|x| x.into_iter().map(TryInto::try_into).collect())
    }

    async fn get_coupons(
        &self,
        conn: &mut PgConn,
        batch: &[CreateSubscription],
        tenant_id: &TenantId,
    ) -> StoreResult<Vec<Coupon>> {
        let coupon_ids: Vec<_> = batch
            .iter()
            .filter_map(|x| x.coupons.as_ref())
            .flat_map(|x| &x.coupons)
            .map(|x| x.coupon_id)
            .unique()
            .collect();

        CouponRow::list_by_ids(conn, &coupon_ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(|x| x.into_iter().map(TryInto::try_into).collect())
    }

    async fn get_customers(
        &self,
        conn: &mut PgConn,
        batch: &[CreateSubscription],
        tenant_id: &TenantId,
    ) -> StoreResult<Vec<Customer>> {
        let customer_ids: Vec<_> = batch
            .iter()
            .map(|c| c.subscription.customer_id)
            .unique()
            .collect();

        CustomerRow::list_by_ids(conn, tenant_id, customer_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|s| s.try_into())
            .collect::<Vec<StoreResult<Customer>>>()
            .into_iter()
            .collect::<StoreResult<Vec<Customer>>>()
    }

    async fn get_customer_connection(
        &self,
        conn: &mut PgConn,
        batch: &[CreateSubscription],
        tenant_id: &TenantId,
    ) -> StoreResult<Vec<CustomerConnection>> {
        let customer_ids: Vec<_> = batch
            .iter()
            .map(|c| c.subscription.customer_id)
            .unique()
            .collect();

        let res =
            CustomerConnectionRow::list_connections_by_customer_ids(conn, tenant_id, customer_ids)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(CustomerConnection::from)
                .collect();

        Ok(res)
    }
}
