use crate::domain::add_ons::AddOn;
use crate::domain::coupons::Coupon;
use crate::domain::{
    CreateSubscription, CreateSubscriptionFromQuote, Customer, InvoicingEntityProviderSensitive,
    PlanForSubscription, PriceComponent,
};
use crate::errors::StoreError;
use crate::store::PgConn;
use crate::{StoreResult, services::Services};
use common_domain::ids::{CouponId, CustomerId, PlanVersionId, TenantId};
use diesel_models::add_ons::AddOnRow;
use diesel_models::coupons::CouponRow;
use diesel_models::customers::CustomerRow;
use diesel_models::invoicing_entities::InvoicingEntityProvidersRow;
use diesel_models::plans::PlanRowForSubscription;
use diesel_models::price_components::PriceComponentRow;
use error_stack::Report;
use itertools::Itertools;
use secrecy::SecretString;
use std::collections::HashMap;

#[derive(Debug)]
pub struct SubscriptionCreationContext {
    pub customers: Vec<Customer>,
    pub plans: Vec<PlanForSubscription>,
    pub price_components_by_plan_version: HashMap<PlanVersionId, Vec<PriceComponent>>,
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
}

impl Services {
    pub(crate) async fn gather_subscription_context(
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
        plan_version_ids: &[PlanVersionId],
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
        plan_version_ids: &[PlanVersionId],
    ) -> StoreResult<HashMap<PlanVersionId, Vec<PriceComponent>>> {
        PriceComponentRow::get_by_plan_version_ids(conn, plan_version_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|(k, v)| {
                let converted_vec: Result<Vec<PriceComponent>, _> =
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

        if let Some((id, name)) =
            CustomerRow::find_archived_customer_in_batch(conn, *tenant_id, customer_ids.clone())
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        {
            return Err(StoreError::InvalidArgument(format!(
                "Cannot create subscription for archived customer: {} ({})",
                name, id
            ))
            .into());
        }

        CustomerRow::list_by_ids(conn, tenant_id, customer_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Vec<StoreResult<Customer>>>()
            .into_iter()
            .collect::<StoreResult<Vec<Customer>>>()
    }

    pub(crate) async fn gather_subscription_context_from_quote(
        &self,
        conn: &mut PgConn,
        params: &CreateSubscriptionFromQuote,
        tenant_id: TenantId,
        secret_decoding_key: &SecretString,
    ) -> StoreResult<SubscriptionCreationContext> {
        let plan_version_ids = vec![params.subscription.plan_version_id];

        let plans = self.get_plans(conn, &plan_version_ids).await?;

        // Get coupons by IDs
        let coupons = self
            .get_coupons_by_ids(conn, &params.coupon_ids, &tenant_id)
            .await?;

        // Get customer
        let customers = self
            .get_customers_by_ids(conn, &[params.subscription.customer_id], &tenant_id)
            .await?;

        let invoicing_entities = self
            .list_invoicing_entities(conn, &tenant_id, secret_decoding_key)
            .await?;

        Ok(SubscriptionCreationContext {
            customers,
            plans,
            price_components_by_plan_version: HashMap::new(), // Not needed for quote conversion
            all_add_ons: vec![],                              // Not needed for quote conversion
            all_coupons: coupons,
            invoicing_entity_providers: invoicing_entities,
        })
    }

    async fn get_coupons_by_ids(
        &self,
        conn: &mut PgConn,
        coupon_ids: &[CouponId],
        tenant_id: &TenantId,
    ) -> StoreResult<Vec<Coupon>> {
        if coupon_ids.is_empty() {
            return Ok(vec![]);
        }

        CouponRow::list_by_ids(conn, coupon_ids, tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(|x| x.into_iter().map(TryInto::try_into).collect())
    }

    async fn get_customers_by_ids(
        &self,
        conn: &mut PgConn,
        customer_ids: &[CustomerId],
        tenant_id: &TenantId,
    ) -> StoreResult<Vec<Customer>> {
        if let Some((id, name)) =
            CustomerRow::find_archived_customer_in_batch(conn, *tenant_id, customer_ids.to_vec())
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        {
            return Err(StoreError::InvalidArgument(format!(
                "Cannot create subscription for archived customer: {} ({})",
                name, id
            ))
            .into());
        }

        CustomerRow::list_by_ids(conn, tenant_id, customer_ids.to_vec())
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Vec<StoreResult<Customer>>>()
            .into_iter()
            .collect::<StoreResult<Vec<Customer>>>()
    }
}
