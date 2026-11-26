use crate::domain::billable_metrics::{
    normalize_usage_breakdown_key, validate_usage_breakdown_key,
};
use crate::domain::entity_activity::Actor;
use crate::domain::outbox_event::OutboxEvent;
use crate::domain::{
    BillableMetric, BillableMetricMeta, BillableMetricNew, MetricAttribution, MetricUnitLabel,
    PaginatedVec, PaginationRequest,
};
use crate::errors::StoreError;
use crate::repositories::subscriptions::usage_attribution::PricingSubscription;
use crate::{Store, StoreResult, domain};
use common_domain::identifiers::validate_code;
use common_domain::ids::{BaseId, BillableMetricId, ProductFamilyId, TenantId};
use common_eventbus::Event;
use diesel::{ExpressionMethods, QueryDsl};
use diesel_models::PgConn;
use diesel_models::billable_metrics::{BillableMetricRow, BillableMetricRowNew};
use diesel_models::customer_credit::CreditRecomputeMarkRow;
use diesel_models::product_families::ProductFamilyRow;
use diesel_models::resources::ResourceTypeRow;
use error_stack::{Report, bail};
use scoped_futures::ScopedFutureExt;

#[async_trait::async_trait]
pub trait BillableMetricInterface {
    async fn find_billable_metric_by_id(
        &self,
        id: BillableMetricId,
        tenant_id: TenantId,
    ) -> StoreResult<domain::BillableMetric>;

    async fn find_billable_metrics_by_ids(
        &self,
        ids: &[BillableMetricId],
        tenant_id: TenantId,
    ) -> StoreResult<Vec<domain::BillableMetric>>;

    async fn list_billable_metrics(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        product_family_id: Option<ProductFamilyId>,
        archived: Option<bool>,
        order_by: Option<String>,
        search: Option<String>,
    ) -> StoreResult<PaginatedVec<domain::BillableMetricMeta>>;

    /// The tenant's metrics after `after` in id order, at most `limit`, whole: a keyset page of
    /// API v2.
    async fn list_billable_metrics_after(
        &self,
        tenant_id: TenantId,
        product_family_id: Option<ProductFamilyId>,
        include_archived: bool,
        after: Option<BillableMetricId>,
        limit: usize,
    ) -> StoreResult<Vec<domain::BillableMetric>>;

    async fn insert_billable_metric(
        &self,
        actor: Actor,
        billable_metric: domain::BillableMetricNew,
    ) -> StoreResult<domain::BillableMetric>;

    /// The tenant's metrics reading the usage events of `code`, archived ones included.
    async fn list_billable_metrics_by_code(
        &self,
        tenant_id: TenantId,
        code: String,
    ) -> StoreResult<Vec<BillableMetric>>;

    /// The tenant's metric of catalog code `code` (API v2's handle), archived ones included.
    async fn find_billable_metric_by_catalog_code(
        &self,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<BillableMetric>;

    /// List all non-archived billable metrics for a tenant (full objects, no pagination).
    async fn list_active_billable_metrics(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<BillableMetric>>;

    async fn archive_billable_metric(
        &self,
        actor: Actor,
        id: BillableMetricId,
        tenant_id: TenantId,
    ) -> StoreResult<()>;

    async fn unarchive_billable_metric(
        &self,
        id: BillableMetricId,
        tenant_id: TenantId,
    ) -> StoreResult<()>;

    async fn update_billable_metric(
        &self,
        actor: Actor,
        id: BillableMetricId,
        tenant_id: TenantId,
        update: domain::BillableMetricUpdate,
    ) -> StoreResult<domain::BillableMetric>;
}

#[async_trait::async_trait]
impl BillableMetricInterface for Store {
    async fn find_billable_metric_by_id(
        &self,
        id: BillableMetricId,
        tenant_id: TenantId,
    ) -> StoreResult<domain::BillableMetric> {
        let mut conn = self.get_conn().await?;

        BillableMetricRow::find_by_id(&mut conn, id, tenant_id)
            .await
            .map_err(Into::into)
            .and_then(TryInto::try_into)
    }

    async fn find_billable_metrics_by_ids(
        &self,
        ids: &[BillableMetricId],
        tenant_id: TenantId,
    ) -> StoreResult<Vec<domain::BillableMetric>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let mut conn = self.get_conn().await?;

        BillableMetricRow::get_by_ids(&mut conn, ids, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }

    async fn list_billable_metrics(
        &self,
        tenant_id: TenantId,
        pagination: PaginationRequest,
        product_family_id: Option<ProductFamilyId>,
        archived: Option<bool>,
        order_by: Option<String>,
        search: Option<String>,
    ) -> StoreResult<PaginatedVec<BillableMetricMeta>> {
        let mut conn = self.get_conn().await?;

        let rows = BillableMetricRow::list(
            &mut conn,
            tenant_id,
            pagination.into(),
            product_family_id,
            archived,
            order_by.as_deref(),
            search,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<BillableMetricMeta> = PaginatedVec {
            items: rows
                .items
                .into_iter()
                .map(std::convert::Into::into)
                .collect(),
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        };

        Ok(res)
    }

    async fn list_billable_metrics_after(
        &self,
        tenant_id: TenantId,
        product_family_id: Option<ProductFamilyId>,
        include_archived: bool,
        after: Option<BillableMetricId>,
        limit: usize,
    ) -> StoreResult<Vec<domain::BillableMetric>> {
        let mut conn = self.get_conn().await?;
        BillableMetricRow::list_page(
            &mut conn,
            tenant_id,
            product_family_id,
            include_archived,
            after,
            i64::try_from(limit).unwrap_or(i64::MAX),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(TryInto::try_into)
        .collect()
    }

    async fn insert_billable_metric(
        &self,
        actor: Actor,
        billable_metric: BillableMetricNew,
    ) -> StoreResult<BillableMetric> {
        validate_code(&billable_metric.code)
            .map_err(|e| Report::new(StoreError::InvalidArgument(e.to_string())))?;
        if let Some(code) = billable_metric.catalog_code.as_deref() {
            crate::domain::catalog_codes::validate_code(code, BillableMetricId::PREFIX)?;
        }
        let usage_breakdown_key =
            normalize_usage_breakdown_key(billable_metric.usage_breakdown_key);
        validate_usage_breakdown_key(
            usage_breakdown_key.as_deref(),
            billable_metric.usage_group_key.as_deref(),
            billable_metric.segmentation_matrix.as_ref(),
        )
        .map_err(|e| Report::new(StoreError::InvalidArgument(e)))?;

        let mut conn = self.get_conn().await?;

        let family = ProductFamilyRow::find_by_id(
            &mut conn,
            billable_metric.product_family_id,
            billable_metric.tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
        crate::repositories::product_families::ensure_family_live(&family)?;

        if let Some(attribution) = &billable_metric.attributed_to {
            validate_attribution(&mut conn, billable_metric.tenant_id, attribution).await?;
        }
        let (attributed_to_type_id, attribution_property) =
            attribution_columns(billable_metric.attributed_to);

        let (unit_singular, unit_plural) = billable_metric
            .unit_label
            .map(MetricUnitLabel::into_parts)
            .unzip();
        let insertable_entity = BillableMetricRowNew {
            id: BillableMetricId::new(),
            name: billable_metric.name,
            description: billable_metric.description,
            code: billable_metric.code,
            aggregation_type: billable_metric.aggregation_type.into(),
            aggregation_key: billable_metric.aggregation_key,
            unit_conversion_factor: billable_metric.unit_conversion_factor,
            unit_conversion_rounding: billable_metric.unit_conversion_rounding.map(Into::into),
            segmentation_matrix: billable_metric
                .segmentation_matrix
                .map(|x| {
                    serde_json::to_value(&x).map_err(|e| {
                        StoreError::SerdeError(
                            "Failed to serialize segmentation_matrix".to_string(),
                            e,
                        )
                    })
                })
                .transpose()?,
            filters: billable_metric
                .filters
                .map(|x| {
                    serde_json::to_value(&x).map_err(|e| {
                        StoreError::SerdeError("Failed to serialize filters".to_string(), e)
                    })
                })
                .transpose()?,
            usage_group_key: billable_metric.usage_group_key,
            usage_breakdown_key,
            unit_singular,
            unit_plural,
            tenant_id: billable_metric.tenant_id,
            product_family_id: family.id,
            product_id: billable_metric.product_id,
            catalog_code: billable_metric.catalog_code,
            attributed_to_type_id,
            attribution_property,
        };

        let tenant_id = insertable_entity.tenant_id;
        let res: BillableMetric = self
            .transaction_with(&mut conn, |conn| {
                let actor = &actor;
                async move {
                    let res: BillableMetric = insertable_entity
                        .insert(conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)
                        .and_then(TryInto::try_into)?;

                    self.internal
                        .record_outbox_batch_tx(
                            conn,
                            tenant_id,
                            actor,
                            vec![OutboxEvent::billable_metric_created(res.clone().into())],
                        )
                        .await?;

                    Ok(res)
                }
                .scope_boxed()
            })
            .await?;

        let _ = self
            .eventbus
            .publish(Event::billable_metric_created(actor, res.id, res.tenant_id))
            .await;

        Ok(res)
    }

    async fn list_billable_metrics_by_code(
        &self,
        tenant_id: TenantId,
        code: String,
    ) -> StoreResult<Vec<BillableMetric>> {
        let mut conn = self.get_conn().await?;

        BillableMetricRow::list_by_code(&mut conn, &tenant_id, code.as_str())
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }

    async fn find_billable_metric_by_catalog_code(
        &self,
        tenant_id: TenantId,
        code: &str,
    ) -> StoreResult<BillableMetric> {
        let mut conn = self.get_conn().await?;

        BillableMetricRow::find_by_catalog_code(&mut conn, tenant_id, code)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| Report::new(StoreError::ValueNotFound(format!("metric {code}"))))
            .and_then(TryInto::try_into)
    }

    async fn list_active_billable_metrics(
        &self,
        tenant_id: TenantId,
    ) -> StoreResult<Vec<BillableMetric>> {
        let mut conn = self.get_conn().await?;

        BillableMetricRow::list_active(&mut conn, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(TryInto::try_into)
            .collect()
    }

    async fn archive_billable_metric(
        &self,
        actor: Actor,
        id: BillableMetricId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.transaction_bumping_tenant(tenant_id, |conn| {
            let actor = &actor;
            async move {
                BillableMetricRow::archive(conn, id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let metric: BillableMetric = BillableMetricRow::find_by_id(conn, id, tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                    .and_then(TryInto::try_into)?;

                self.internal
                    .record_outbox_batch_tx(
                        conn,
                        tenant_id,
                        actor,
                        vec![OutboxEvent::billable_metric_archived(metric.into())],
                    )
                    .await?;

                Ok(())
            }
            .scope_boxed()
        })
        .await
    }

    async fn unarchive_billable_metric(
        &self,
        id: BillableMetricId,
        tenant_id: TenantId,
    ) -> StoreResult<()> {
        self.transaction_bumping_tenant(tenant_id, |conn| {
            async move { Ok(BillableMetricRow::unarchive(conn, id, tenant_id).await?) }
                .scope_boxed()
        })
        .await
    }

    async fn update_billable_metric(
        &self,
        actor: Actor,
        id: BillableMetricId,
        tenant_id: TenantId,
        update: domain::BillableMetricUpdate,
    ) -> StoreResult<domain::BillableMetric> {
        use diesel_models::billable_metrics::BillableMetricRowPatch;

        let (unit_singular, unit_plural) = match update.unit_label {
            None => (None, None),
            Some(None) => (Some(None), Some(None)),
            Some(Some(label)) => {
                let (singular, plural) = label.into_parts();
                (Some(Some(singular)), Some(Some(plural)))
            }
        };
        let attributed_to = update.attributed_to;
        let (attributed_to_type_id, attribution_property) = attributed_to
            .clone()
            .map(attribution_columns)
            .map_or((None, None), |(t, p)| (Some(t), Some(p)));
        let patch = BillableMetricRowPatch {
            name: update.name,
            description: update.description,
            unit_conversion_factor: update.unit_conversion_factor,
            unit_conversion_rounding: update.unit_conversion_rounding.map(|x| x.map(Into::into)),
            segmentation_matrix: update
                .segmentation_matrix
                .map(|opt| {
                    opt.map(|x| {
                        serde_json::to_value(&x).map_err(|e| {
                            StoreError::SerdeError(
                                "Failed to serialize segmentation_matrix".to_string(),
                                e,
                            )
                        })
                    })
                    .transpose()
                })
                .transpose()?,
            filters: update
                .filters
                .map(|opt| {
                    opt.map(|x| {
                        serde_json::to_value(&x).map_err(|e| {
                            StoreError::SerdeError("Failed to serialize filters".to_string(), e)
                        })
                    })
                    .transpose()
                })
                .transpose()?,
            unit_singular,
            unit_plural,
            usage_breakdown_key: update
                .usage_breakdown_key
                .map(normalize_usage_breakdown_key),
            updated_at: Some(common_clock::now().naive_utc()),
            attributed_to_type_id,
            attribution_property,
        };

        self.transaction_bumping_tenant(tenant_id, |conn| {
            let actor = &actor;
            let attributed_to = &attributed_to;
            async move {
                if let Some(new) = attributed_to {
                    let before: BillableMetric = BillableMetricRow::find_by_id(conn, id, tenant_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)
                        .and_then(TryInto::try_into)?;
                    if let Some(attribution) = new {
                        validate_attribution(conn, tenant_id, attribution).await?;
                    }
                    if before.attribution() != *new {
                        let pricing = crate::repositories::subscriptions::usage_attribution::priced_for_a_resource(
                            conn, tenant_id, id,
                        )
                        .await?;
                        refuse_while_priced_for_a_resource(conn, tenant_id, &pricing).await?;
                        mark_attribution_change(conn, tenant_id, &pricing).await?;
                    }
                }
                let metric: BillableMetric = BillableMetricRow::update(conn, id, tenant_id, patch)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                    .and_then(TryInto::try_into)?;
                validate_usage_breakdown_key(
                    metric.usage_breakdown_key.as_deref(),
                    metric.usage_group_key.as_deref(),
                    metric.segmentation_matrix.as_ref(),
                )
                .map_err(|e| Report::new(StoreError::InvalidArgument(e)))?;

                self.internal
                    .record_outbox_batch_tx(
                        conn,
                        tenant_id,
                        actor,
                        vec![OutboxEvent::billable_metric_updated(metric.clone().into())],
                    )
                    .await?;

                Ok(metric)
            }
            .scope_boxed()
        })
        .await
    }
}

fn attribution_columns(
    attribution: Option<MetricAttribution>,
) -> (Option<common_domain::ids::ResourceTypeId>, Option<String>) {
    attribution.map_or((None, None), |a| (Some(a.type_id), a.property))
}

pub(crate) const MAX_ATTRIBUTION_PROPERTY_LEN: usize = 256;

/// The type exists and is not archived; a set property is a plain event property name.
pub(crate) async fn validate_attribution(
    conn: &mut PgConn,
    tenant_id: TenantId,
    attribution: &MetricAttribution,
) -> StoreResult<()> {
    let resource_type = ResourceTypeRow::find_by_id(conn, tenant_id, attribution.type_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    if resource_type.archived_at.is_some() {
        bail!(StoreError::InvalidArgument(format!(
            "resource type {} is archived",
            resource_type.code
        )));
    }
    if let Some(property) = &attribution.property
        && (property.trim().is_empty()
            || property.trim() != property
            || property.len() > MAX_ATTRIBUTION_PROPERTY_LEN)
    {
        bail!(StoreError::InvalidArgument(format!(
            "the attribution property must be a non-blank name of at most \
             {MAX_ATTRIBUTION_PROPERTY_LEN} bytes, without surrounding whitespace"
        )));
    }
    Ok(())
}

/// A change would move usage between a subscription for a resource and its customer's other
/// subscriptions over periods they already finalized, or that its own draft still bills.
/// Attribution is not versioned: once one of them has an invoice, it is fixed (§3.6).
async fn refuse_while_priced_for_a_resource(
    conn: &mut PgConn,
    tenant_id: TenantId,
    pricing: &[PricingSubscription],
) -> StoreResult<()> {
    use diesel_async::RunQueryDsl;
    if pricing.iter().any(|s| s.live) {
        bail!(StoreError::InvalidArgument(
            "a subscription for a resource prices this metric, so its attribution can't change \
             while one is live: create a new metric instead"
                .to_owned()
        ));
    }
    #[derive(diesel::QueryableByName)]
    struct Invoices {
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        invoices: i64,
    }
    let invoices = diesel::sql_query(
        "SELECT count(*) AS invoices
         FROM invoice i
         LEFT JOIN invoice parent ON parent.id = i.consolidated_into_invoice_id
         WHERE i.tenant_id = $1 AND i.subscription_id = ANY($2)
           AND COALESCE(parent.status, i.status) <> 'VOID'",
    )
    .bind::<diesel::sql_types::Uuid, _>(tenant_id.as_uuid())
    .bind::<diesel::sql_types::Array<diesel::sql_types::Uuid>, _>(
        pricing.iter().map(|s| s.id.as_uuid()).collect::<Vec<_>>(),
    )
    .get_result::<Invoices>(conn)
    .await
    .map_err(|e| Report::new(StoreError::from(e)))?
    .invoices;
    if invoices > 0 {
        bail!(StoreError::InvalidArgument(
            "a subscription for a resource that priced this metric has invoiced it, or has an \
             invoice in draft: its attribution can no longer change, create a new metric instead"
                .to_owned()
        ));
    }
    Ok(())
}

/// The drafts [`mark_attribution_change`] refreshes for `pricing`, counted with its filter.
pub(crate) async fn attribution_drafts(
    conn: &mut PgConn,
    tenant_id: TenantId,
    pricing: &[PricingSubscription],
) -> StoreResult<i64> {
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::invoice::dsl as i;
    let customers = pricing
        .iter()
        .map(|s| s.customer_id)
        .collect::<std::collections::HashSet<_>>();
    if customers.is_empty() {
        return Ok(0);
    }
    i::invoice
        .filter(i::tenant_id.eq(tenant_id))
        .filter(i::status.eq(diesel_models::enums::InvoiceStatusEnum::Draft))
        .filter(i::subscription_id.is_not_null())
        .filter(i::consolidated_into_invoice_id.is_null())
        .filter(i::customer_id.eq_any(customers.into_iter().collect::<Vec<_>>()))
        .count()
        .get_result(conn)
        .await
        .map_err(|e| Report::new(StoreError::from(e)))
}

/// A change of attribution moves usage of the customers whose subscriptions for a resource priced
/// the metric: their credit projection is recomputed and their drafts refreshed by the draft worker.
async fn mark_attribution_change(
    conn: &mut PgConn,
    tenant_id: TenantId,
    pricing: &[PricingSubscription],
) -> StoreResult<()> {
    use diesel_async::RunQueryDsl;
    use diesel_models::schema::invoice::dsl as i;
    let customers = pricing
        .iter()
        .map(|s| s.customer_id)
        .collect::<std::collections::HashSet<_>>();
    if customers.is_empty() {
        return Ok(());
    }
    CreditRecomputeMarkRow::upsert_many(
        conn,
        &customers
            .iter()
            .map(|customer_id| (tenant_id, *customer_id))
            .collect::<Vec<_>>(),
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)?;
    diesel::update(
        i::invoice
            .filter(i::tenant_id.eq(tenant_id))
            .filter(i::status.eq(diesel_models::enums::InvoiceStatusEnum::Draft))
            .filter(i::subscription_id.is_not_null())
            .filter(i::consolidated_into_invoice_id.is_null())
            .filter(i::customer_id.eq_any(customers.into_iter().collect::<Vec<_>>())),
    )
    .set(i::data_updated_at.eq(None::<chrono::NaiveDateTime>))
    .execute(conn)
    .await
    .map_err(|e| Report::new(StoreError::from(e)))?;
    Ok(())
}
