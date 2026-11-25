//! Finding the catalog by code and subscriptions by external id (ADR-0013). Creating under a code
//! is each resource's own (`create_product_with_code`, `create_add_on_with_code`,
//! `insert_product_family_with_code`, `PlanDraftsInterface::create_plan_with_draft`).
use crate::StoreResult;
use crate::errors::StoreError;
use crate::store::Store;
use common_domain::ids::{
    AddOnId, BaseId, BillableMetricId, CouponId, CreditNoteId, CreditUnitId, CustomerId, FeatureId,
    InvoiceId, PlanId, ProductFamilyId, ProductId, SubscriptionId, TenantId,
};
use diesel_models::query::catalog_codes;
use error_stack::Report;
use std::collections::HashMap;

/// The resources a response references, to read their human keys in one round
/// ([`CatalogCodesInterface::reference_keys`]): a list reads those of its whole page at once.
#[derive(Debug, Clone, Default)]
pub struct ReferenceIds {
    pub customers: Vec<CustomerId>,
    pub subscriptions: Vec<SubscriptionId>,
    pub plans: Vec<PlanId>,
    pub products: Vec<ProductId>,
    pub add_ons: Vec<AddOnId>,
    pub product_families: Vec<ProductFamilyId>,
    pub metrics: Vec<BillableMetricId>,
    pub credit_units: Vec<CreditUnitId>,
    /// Credit units named by code, as wallets and grants name them.
    pub credit_unit_codes: Vec<String>,
    pub coupons: Vec<CouponId>,
    pub features: Vec<FeatureId>,
    pub invoices: Vec<InvoiceId>,
    pub credit_notes: Vec<CreditNoteId>,
}

/// A customer's keys: its alias and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomerKeys {
    pub alias: Option<String>,
    pub name: String,
}

/// The human keys of [`ReferenceIds`]: codes, a subscription's external id, a document's number.
/// A resource not the tenant's is absent.
#[derive(Debug, Clone, Default)]
pub struct ReferenceKeys {
    pub customers: HashMap<CustomerId, CustomerKeys>,
    pub subscriptions: HashMap<SubscriptionId, Option<String>>,
    pub plans: HashMap<PlanId, String>,
    pub products: HashMap<ProductId, String>,
    pub add_ons: HashMap<AddOnId, String>,
    pub product_families: HashMap<ProductFamilyId, String>,
    pub metrics: HashMap<BillableMetricId, String>,
    pub credit_units: HashMap<CreditUnitId, String>,
    /// The ids of the credit units named by code.
    pub credit_unit_ids: HashMap<String, CreditUnitId>,
    pub coupons: HashMap<CouponId, String>,
    pub features: HashMap<FeatureId, String>,
    pub invoices: HashMap<InvoiceId, String>,
    pub credit_notes: HashMap<CreditNoteId, String>,
}

fn distinct<ID: BaseId<IdType = ID> + Copy>(ids: &[ID]) -> Vec<ID> {
    let mut ids = ids.to_vec();
    ids.sort_by_key(|id| id.as_uuid());
    ids.dedup_by_key(|id| id.as_uuid());
    ids
}

fn not_found(what: &str, code: &str) -> Report<StoreError> {
    Report::new(StoreError::ValueNotFound(format!("{what} {code}")))
}

#[async_trait::async_trait]
pub trait CatalogCodesInterface {
    /// The tenant's plan of `code`, archived ones included.
    async fn find_plan_id_by_code(&self, tenant_id: TenantId, code: &str) -> StoreResult<PlanId>;

    async fn find_product_id_by_code(
        &self,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<ProductId>;

    async fn find_add_on_id_by_code(&self, tenant_id: TenantId, code: &str)
    -> StoreResult<AddOnId>;

    async fn find_product_family_id_by_code(
        &self,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<ProductFamilyId>;

    /// The codes of the tenant's plans of `ids`; one not the tenant's is absent.
    async fn plan_codes(
        &self,
        tenant_id: TenantId,
        ids: &[PlanId],
    ) -> StoreResult<HashMap<PlanId, String>>;

    async fn find_subscription_id_by_external_id(
        &self,
        tenant_id: TenantId,
        external_id: &str,
    ) -> StoreResult<SubscriptionId>;

    /// Sets or clears a subscription's external id; one another subscription of the tenant has
    /// is a duplicate.
    async fn set_subscription_external_id(
        &self,
        tenant_id: TenantId,
        id: SubscriptionId,
        external_id: Option<String>,
    ) -> StoreResult<()>;

    /// The human keys of what `ids` names, one query per kind named, on one connection.
    async fn reference_keys(
        &self,
        tenant_id: TenantId,
        ids: &ReferenceIds,
    ) -> StoreResult<ReferenceKeys>;
}

#[async_trait::async_trait]
impl CatalogCodesInterface for Store {
    async fn find_plan_id_by_code(&self, tenant_id: TenantId, code: &str) -> StoreResult<PlanId> {
        let mut conn = self.get_conn().await?;
        catalog_codes::plan_id_by_code(&mut conn, tenant_id, code)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| not_found("plan", code))
    }

    async fn find_product_id_by_code(
        &self,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<ProductId> {
        let mut conn = self.get_conn().await?;
        catalog_codes::product_id_by_code(&mut conn, tenant_id, code)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| not_found("product", code))
    }

    async fn find_add_on_id_by_code(
        &self,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<AddOnId> {
        let mut conn = self.get_conn().await?;
        catalog_codes::add_on_id_by_code(&mut conn, tenant_id, code)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| not_found("add-on", code))
    }

    async fn find_product_family_id_by_code(
        &self,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<ProductFamilyId> {
        let mut conn = self.get_conn().await?;
        catalog_codes::product_family_id_by_code(&mut conn, tenant_id, code)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| not_found("product family", code))
    }

    async fn plan_codes(
        &self,
        tenant_id: TenantId,
        ids: &[PlanId],
    ) -> StoreResult<HashMap<PlanId, String>> {
        let mut conn = self.get_conn().await?;
        Ok(catalog_codes::plan_codes(&mut conn, tenant_id, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .collect())
    }

    async fn find_subscription_id_by_external_id(
        &self,
        tenant_id: TenantId,
        external_id: &str,
    ) -> StoreResult<SubscriptionId> {
        let mut conn = self.get_conn().await?;
        catalog_codes::subscription_id_by_external_id(&mut conn, tenant_id, external_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| not_found("subscription", external_id))
    }

    async fn set_subscription_external_id(
        &self,
        tenant_id: TenantId,
        id: SubscriptionId,
        external_id: Option<String>,
    ) -> StoreResult<()> {
        if let Some(external_id) = external_id.as_deref()
            && (external_id.trim().is_empty() || external_id.len() > 255)
        {
            return Err(Report::new(StoreError::InvalidArgument(
                "an external id is 1 to 255 bytes".to_owned(),
            )));
        }
        let mut conn = self.get_conn().await?;
        let updated = catalog_codes::set_subscription_external_id(
            &mut conn,
            tenant_id,
            id,
            external_id.as_deref(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        if updated == 0 {
            return Err(not_found("subscription", &id.to_string()));
        }
        Ok(())
    }

    async fn reference_keys(
        &self,
        tenant_id: TenantId,
        ids: &ReferenceIds,
    ) -> StoreResult<ReferenceKeys> {
        macro_rules! keys {
            ($conn:ident, $find:ident, $ids:expr) => {{
                let ids = distinct($ids);
                if ids.is_empty() {
                    HashMap::new()
                } else {
                    catalog_codes::$find(&mut $conn, tenant_id, &ids)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .into_iter()
                        .collect()
                }
            }};
        }
        let mut conn = self.get_conn().await?;
        let customer_ids = distinct(&ids.customers);
        let customers = if customer_ids.is_empty() {
            HashMap::new()
        } else {
            catalog_codes::customer_keys(&mut conn, tenant_id, &customer_ids)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|(id, alias, name)| (id, CustomerKeys { alias, name }))
                .collect()
        };
        let mut codes = ids.credit_unit_codes.clone();
        codes.sort();
        codes.dedup();
        let credit_unit_ids = if codes.is_empty() {
            HashMap::new()
        } else {
            catalog_codes::credit_unit_ids(&mut conn, tenant_id, &codes)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .collect()
        };
        Ok(ReferenceKeys {
            customers,
            credit_unit_ids,
            subscriptions: keys!(conn, subscription_external_ids, &ids.subscriptions),
            plans: keys!(conn, plan_codes, &ids.plans),
            products: keys!(conn, product_codes, &ids.products),
            add_ons: keys!(conn, add_on_codes, &ids.add_ons),
            product_families: keys!(conn, product_family_codes, &ids.product_families),
            metrics: keys!(conn, metric_codes, &ids.metrics),
            credit_units: keys!(conn, credit_unit_codes, &ids.credit_units),
            coupons: keys!(conn, coupon_codes, &ids.coupons),
            features: keys!(conn, feature_codes, &ids.features),
            invoices: keys!(conn, invoice_numbers, &ids.invoices),
            credit_notes: keys!(conn, credit_note_numbers, &ids.credit_notes),
        })
    }
}
