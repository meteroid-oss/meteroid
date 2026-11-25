use crate::StoreResult;
use crate::domain::credit_units::CreditUnitsById;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::enums::SubscriptionFeeBillingPeriod;
use crate::domain::outbox_event::{OutboxEvent, SubscriptionEvent, SubscriptionUpdateType};
use crate::domain::prices::{Price, fee_type_billing_period, resolve_subscription_fee};
use crate::domain::products::Product;
use crate::domain::subscriptions::{AmendmentSummary, PaymentMethodsConfig, PendingScheduledEvent};
use crate::domain::{
    BillableMetric, ConnectorProviderEnum, Customer, InvoicingEntity, PaginatedVec,
    PaginationRequest, Schedule, Subscription, SubscriptionComponent, SubscriptionDetails,
    SubscriptionPatch, TrialConfig, UsageFilter,
};
use crate::repositories::credit_units::load_credit_units;
use chrono::NaiveDate;
use common_domain::ids::{
    BaseId, ConnectorId, CreditUnitId, CustomerId, EntitlementEntityId, PlanId, PriceId, ProductId,
    SubscriptionId, TenantId,
};
use std::collections::HashMap;

use crate::errors::StoreError;
use crate::services::validate_charge_automatically_with_provider_ids;
use crate::store::Store;
use error_stack::{Report, bail};
use itertools::Itertools;

use crate::domain::entitlements::Entitlement;
use crate::domain::subscription_add_ons::SubscriptionAddOn;
use diesel_models::applied_coupons::AppliedCouponDetailedRow;
use diesel_models::billable_metrics::BillableMetricRow;
use diesel_models::entitlements::EntitlementRow;
use diesel_models::prices::PriceRow;
use diesel_models::products::ProductRow;
use diesel_models::schedules::ScheduleRow;
use diesel_models::subscription_add_ons::SubscriptionAddOnRow;
use diesel_models::subscription_components::SubscriptionComponentRow;
use diesel_models::subscriptions::{SubscriptionRow, SubscriptionRowPatch};
// TODO we need to always pass the tenant id and match it with the resource, if not within the resource.
// and even within it's probably still unsafe no ? Ex: creating components against a wrong subscription within a different tenant
use crate::domain::pgmq::{HubspotSyncRequestEvent, HubspotSyncSubscription, PgmqQueue};
use crate::repositories::connectors::ConnectorsInterface;
use crate::repositories::pgmq::PgmqInterface;
use diesel_models::PgConn;
use diesel_models::customers::CustomerRow;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::plans::PlanRow;
use meteroid_store_macros::with_conn_delegate;

pub mod slots;
pub(crate) mod usage_attribution;
use crate::domain::scheduled_events::{ScheduledEvent, ScheduledEventData};
use diesel_models::subscription_changes::SubscriptionChangeRow;
use diesel_models::subscription_facts::SubscriptionEndingRow;
pub use slots::SubscriptionSlotsInterface;

pub enum CancellationEffectiveAt {
    EndOfBillingPeriod,
    Date(NaiveDate),
    /// Terminate now: settles the current period through the actual instant, so usage consumed on
    /// the termination day is billed rather than dropped by the date-granular midnight window.
    Immediate,
}

#[with_conn_delegate]
pub trait SubscriptionInterface {
    #[delegated]
    async fn get_subscription_details(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<SubscriptionDetails>;

    #[delegated]
    async fn get_subscription_details_minimal(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<SubscriptionDetails>;

    async fn get_subscription(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Subscription>;

    async fn list_subscriptions(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        plan_id: Option<PlanId>,
        status: Option<Vec<crate::domain::enums::SubscriptionStatusEnum>>,
        search: Option<String>,
        filters: Vec<crate::domain::filters::Filter>,
        order_by: Option<String>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<Subscription>>;

    /// Map of subscription_id -> next scheduled cancellation date, for the given subscriptions.
    /// Only subscriptions with a pending cancellation are present in the map.
    async fn list_pending_cancellations(
        &self,
        tenant_id: TenantId,
        subscription_ids: &[SubscriptionId],
    ) -> StoreResult<HashMap<SubscriptionId, NaiveDate>>;

    async fn patch_subscription_conn_meta(
        &self,
        subscription_id: SubscriptionId,
        connector_id: ConnectorId,
        provider: ConnectorProviderEnum,
        external_id: &str,
        external_company_id: &str,
    ) -> StoreResult<()>;

    async fn sync_subscriptions_to_hubspot(
        &self,
        tenant_id: TenantId,
        subscription_ids: Vec<SubscriptionId>,
    ) -> StoreResult<()>;

    async fn sync_customer_subscriptions_to_hubspot(
        &self,
        tenant_id: TenantId,
        customer_ids: Vec<CustomerId>,
    ) -> StoreResult<()>;

    async fn list_subscription_by_ids_global(
        &self,
        subscription_ids: Vec<SubscriptionId>,
    ) -> StoreResult<Vec<Subscription>>;

    async fn patch_subscription(
        &self,
        tenant_id: TenantId,
        patch: SubscriptionPatch,
        actor: Actor,
    ) -> StoreResult<Subscription>;
}

#[async_trait::async_trait]
impl SubscriptionInterface for Store {
    async fn get_subscription(
        &self,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<Subscription> {
        let mut conn = self.get_conn().await?;

        let db_subscription =
            SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        db_subscription.try_into()
    }

    async fn get_subscription_details_minimal_with_conn(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<SubscriptionDetails> {
        let subscription: Subscription =
            SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .try_into()?;

        let customer: Customer =
            CustomerRow::find_by_id(conn, &subscription.customer_id, &tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .try_into()?;

        let invoicing_entity: InvoicingEntity =
            InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                conn,
                customer.invoicing_entity_id,
                tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into();

        let usage_scopes =
            usage_attribution::subscription_usage_scopes(conn, &subscription).await?;

        Ok(SubscriptionDetails {
            subscription,
            customer,
            invoicing_entity,
            schedules: vec![],
            price_components: vec![],
            add_ons: vec![],
            applied_coupons: vec![],
            metrics: vec![],
            usage_scopes,
            checkout_url: None,
            trial_config: None,
            pending_events: vec![],
            entitlements: vec![],
            minimum: None,
            plan_option: None,
        })
    }

    /// todo optimize db calls
    async fn get_subscription_details_with_conn(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
    ) -> StoreResult<SubscriptionDetails> {
        // Today, held inside the stored period: the rows a change wrote for its end are the next
        // period's, read once the renewal advances it.
        let end = SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .subscription
            .current_period_end;
        let today = common_clock::now().date_naive();
        let day = end
            .and_then(|end| end.pred_opt())
            .map_or(today, |last| today.min(last));
        self.subscription_details_as_of(conn, tenant_id, subscription_id, day)
            .await
    }

    async fn list_subscriptions(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        plan_id: Option<PlanId>,
        status: Option<Vec<crate::domain::enums::SubscriptionStatusEnum>>,
        search: Option<String>,
        filters: Vec<crate::domain::filters::Filter>,
        order_by: Option<String>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<Subscription>> {
        let mut conn = self.get_conn().await?;

        let status_filter = status.map(|s| s.into_iter().map(|x| x.into()).collect());

        let cp_filters = if filters.is_empty() {
            vec![]
        } else {
            let definitions = self
                .internal
                .custom_property_definitions
                .load_active(
                    &mut conn,
                    tenant_id,
                    crate::domain::enums::CustomPropertyEntityType::Subscription,
                )
                .await?;
            crate::repositories::filter_resolution::resolve_list_filters(
                crate::repositories::filter_resolution::ListEntity::Subscription,
                definitions.as_slice(),
                filters,
            )?
        };

        let db_subscriptions = SubscriptionRow::list_subscriptions(
            &mut conn,
            &tenant_id,
            customer_id,
            plan_id,
            status_filter,
            search,
            cp_filters,
            order_by.as_deref(),
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let res: PaginatedVec<Subscription> = PaginatedVec {
            items: db_subscriptions
                .items
                .into_iter()
                .map(std::convert::TryInto::try_into)
                .collect::<Result<Vec<_>, _>>()?,
            total_pages: db_subscriptions.total_pages,
            total_results: db_subscriptions.total_results,
        };

        Ok(res)
    }

    async fn list_pending_cancellations(
        &self,
        tenant_id: TenantId,
        subscription_ids: &[SubscriptionId],
    ) -> StoreResult<HashMap<SubscriptionId, NaiveDate>> {
        if subscription_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let mut conn = self.get_conn().await?;

        let rows = SubscriptionEndingRow::list_pending_cancellations(
            &mut conn,
            tenant_id,
            subscription_ids,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let mut map = HashMap::new();
        for row in rows {
            map.entry(row.subscription_id)
                .or_insert_with(|| row.at.date());
        }

        Ok(map)
    }

    async fn patch_subscription_conn_meta(
        &self,
        subscription_id: SubscriptionId,
        connector_id: ConnectorId,
        provider: ConnectorProviderEnum,
        external_id: &str,
        external_company_id: &str,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        SubscriptionRow::upsert_conn_meta(
            &mut conn,
            provider.into(),
            subscription_id,
            connector_id,
            external_id,
            external_company_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
    }

    async fn sync_subscriptions_to_hubspot(
        &self,
        tenant_id: TenantId,
        subscription_ids: Vec<SubscriptionId>,
    ) -> StoreResult<()> {
        let connector = self.get_hubspot_connector(tenant_id).await?;

        if connector.is_none() {
            bail!(StoreError::InvalidArgument(
                "No Hubspot connector found".to_string()
            ));
        }

        let mut conn = self.get_conn().await?;

        let db_subscriptions =
            SubscriptionRow::list_subscriptions_by_ids(&mut conn, &tenant_id, &subscription_ids)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        self.pgmq_send_batch(
            PgmqQueue::HubspotSync,
            db_subscriptions
                .into_iter()
                .map(|subscription| {
                    HubspotSyncRequestEvent::Subscription(Box::new(HubspotSyncSubscription {
                        id: subscription.subscription.id,
                        tenant_id,
                    }))
                    .try_into()
                })
                .collect::<Result<Vec<_>, _>>()?,
        )
        .await
    }

    async fn list_subscription_by_ids_global(
        &self,
        subscription_ids: Vec<SubscriptionId>,
    ) -> StoreResult<Vec<Subscription>> {
        let mut conn = self.get_conn().await?;

        SubscriptionRow::list_by_ids(&mut conn, &subscription_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn sync_customer_subscriptions_to_hubspot(
        &self,
        tenant_id: TenantId,
        customer_ids: Vec<CustomerId>,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        let req = SubscriptionRow::list_by_customer_ids(&mut conn, tenant_id, &customer_ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(|subscription| {
                HubspotSyncRequestEvent::Subscription(Box::new(HubspotSyncSubscription {
                    id: subscription.subscription.id,
                    tenant_id: subscription.subscription.tenant_id,
                }))
                .try_into()
            })
            .collect::<Result<Vec<_>, _>>()?;

        self.pgmq_send_batch(PgmqQueue::HubspotSync, req).await
    }

    async fn patch_subscription(
        &self,
        tenant_id: TenantId,
        patch: SubscriptionPatch,
        actor: Actor,
    ) -> StoreResult<Subscription> {
        use crate::domain::SubscriptionStatusEnum;

        let mut conn = self.get_conn().await?;

        let existing = SubscriptionRow::get_subscription_by_id(&mut conn, &tenant_id, patch.id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let status: SubscriptionStatusEnum = existing.subscription.status.into();
        if matches!(
            status,
            SubscriptionStatusEnum::Cancelled | SubscriptionStatusEnum::Completed
        ) {
            bail!(StoreError::InvalidArgument(
                "Cannot update subscription in terminal state".to_string()
            ));
        }

        let effective_charge_automatically = patch
            .charge_automatically
            .unwrap_or(existing.subscription.charge_automatically);

        let existing_payment_methods_config: Option<PaymentMethodsConfig> = existing
            .subscription
            .payment_methods_config
            .as_ref()
            .map(|v| serde_json::from_value(v.clone()))
            .transpose()
            .map_err(|e| {
                StoreError::SerdeError(format!("Failed to parse payment_methods_config: {}", e), e)
            })?;

        // Snapshot pre-patch values so the entity.updated audit row records only the
        // fields whose value actually changes — clients round-trip the whole form.
        let before_charge_automatically = existing.subscription.charge_automatically;
        let before_auto_advance_invoices = existing.subscription.auto_advance_invoices;
        let before_net_terms = existing.subscription.net_terms;
        let before_invoice_memo = existing.subscription.invoice_memo.clone();
        let before_purchase_order = existing.subscription.purchase_order.clone();
        let before_payment_methods_config = existing_payment_methods_config.clone();

        let effective_payment_methods_config = match &patch.payment_methods_config {
            Some(new_config) => new_config.clone(),
            None => existing_payment_methods_config,
        };

        if effective_charge_automatically {
            let invoicing_entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                &mut conn,
                existing.invoicing_entity_id,
                tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

            validate_charge_automatically_with_provider_ids(
                effective_charge_automatically,
                effective_payment_methods_config.as_ref(),
                invoicing_entity.card_provider_id,
                invoicing_entity.direct_debit_provider_id,
            )?;
        }

        let custom_properties = match &patch.custom_properties {
            Some(provided) => Some(
                self.internal
                    .validate_and_normalize_custom_properties(
                        &mut conn,
                        tenant_id,
                        crate::domain::enums::CustomPropertyEntityType::Subscription,
                        provided,
                        &existing.subscription.custom_properties,
                        crate::domain::custom_property_definitions::CustomPropertyMode::Merge,
                    )
                    .await?,
            ),
            None => None,
        };

        let row_patch = SubscriptionRowPatch {
            charge_automatically: patch.charge_automatically,
            auto_advance_invoices: patch.auto_advance_invoices,
            net_terms: patch.net_terms.map(|n| n as i32),
            invoice_memo: patch.invoice_memo,
            purchase_order: patch.purchase_order,
            payment_methods_config: patch
                .payment_methods_config
                .map(|opt| {
                    opt.map(serde_json::to_value).transpose().map_err(|e| {
                        StoreError::SerdeError(
                            "Failed to serialize payment_methods_config".to_string(),
                            e,
                        )
                    })
                })
                .transpose()?,
            custom_properties,
            usage_filter_property: None,
            usage_filter_value: None,
            usage_filter_since: None,
        };

        let subscription_id = patch.id;
        let customer_id = existing.subscription.customer_id;
        let before_usage_filter = UsageFilter::from_columns(
            existing.subscription.usage_filter_property.clone(),
            existing.subscription.usage_filter_value.clone(),
        )?;
        let usage_filter_change = patch
            .usage_filter
            .clone()
            .filter(|new| *new != before_usage_filter);

        // Patch and the resulting `subscription.updated` webhook must commit atomically:
        // the transactional outbox guarantees the event is published iff the row changed.
        use scoped_futures::ScopedFutureExt;
        self.transaction(|conn| {
            let row_patch = &row_patch;
            let actor = &actor;
            let before_invoice_memo = &before_invoice_memo;
            let before_purchase_order = &before_purchase_order;
            let before_payment_methods_config = &before_payment_methods_config;
            let before_usage_filter = &before_usage_filter;
            let usage_filter_change = &usage_filter_change;
            async move {
                let mut row_patch = row_patch.clone();
                // Serializes with the customer's other filter changes and creations.
                let lock = crate::repositories::credit::customer_credit::lock_customer_tx(
                    conn,
                    tenant_id,
                    customer_id,
                )
                .await?;
                diesel_models::query::credit_inputs::bump_customer(conn, &lock).await?;
                if let Some(new) = usage_filter_change {
                    let row =
                        SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?
                            .subscription;
                    let from = usage_attribution::usage_filter_effective_from(
                        conn,
                        tenant_id,
                        &row,
                        usage_attribution::midnight(row.current_period_start),
                        new.as_ref(),
                        &std::collections::HashSet::new(),
                    )
                    .await?;
                    if let Some(filter) = new {
                        usage_attribution::ensure_usage_filter_available(
                            conn,
                            tenant_id,
                            customer_id,
                            subscription_id,
                            filter,
                            from,
                            usage_attribution::life_end(&row),
                        )
                        .await?;
                    }
                    usage_attribution::record_usage_filter_version(
                        conn,
                        tenant_id,
                        subscription_id,
                        new.as_ref(),
                        from,
                    )
                    .await?;
                    row_patch.usage_filter_property =
                        Some(new.as_ref().map(|f| f.property.clone()));
                    row_patch.usage_filter_value = Some(new.as_ref().map(|f| f.value.clone()));
                    row_patch.usage_filter_since = Some(Some(from));
                }
                if !row_patch.is_empty() {
                    SubscriptionRow::patch(conn, &tenant_id, subscription_id, &row_patch)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                }

                let updated: Subscription =
                    SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                        .try_into()?;

                let mut changed_fields: Vec<&'static str> = Vec::new();
                if before_charge_automatically != updated.charge_automatically {
                    changed_fields.push("charge_automatically");
                }
                if before_auto_advance_invoices != updated.auto_advance_invoices {
                    changed_fields.push("auto_advance_invoices");
                }
                if before_net_terms != updated.net_terms as i32 {
                    changed_fields.push("net_terms");
                }
                if before_invoice_memo != &updated.invoice_memo {
                    changed_fields.push("invoice_memo");
                }
                if before_purchase_order != &updated.purchase_order {
                    changed_fields.push("purchase_order");
                }
                if before_payment_methods_config != &updated.payment_methods_config {
                    changed_fields.push("payment_methods_config");
                }
                if before_usage_filter != &updated.usage_filter {
                    changed_fields.push("usage_filter");
                }

                if !changed_fields.is_empty() {
                    let activity = Activity::new(
                        ActivityType::EntityUpdated,
                        EntityType::Subscription,
                        subscription_id.as_uuid(),
                    )
                    .agg_customer(updated.customer_id)
                    .with_metadata(serde_json::json!({
                        "changes": changed_fields
                            .iter()
                            .map(|f| serde_json::json!({ "field": f }))
                            .collect::<Vec<_>>(),
                    }));
                    self.internal
                        .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                        .await?;
                }

                let mut event: SubscriptionEvent = updated.clone().into();
                event.change_type = Some(SubscriptionUpdateType::BillingConfigurationUpdated);
                self.internal
                    .record_outbox_batch_tx(
                        conn,
                        tenant_id,
                        actor,
                        vec![OutboxEvent::subscription_updated(event)],
                    )
                    .await?;

                Ok(updated)
            }
            .scope_boxed()
        })
        .await
    }
}

impl Store {
    /// The subscription as its rows in force on `as_of` read: today's, or a renewal's ahead.
    pub(crate) async fn subscription_details_as_of(
        &self,
        conn: &mut PgConn,
        tenant_id: TenantId,
        subscription_id: SubscriptionId,
        as_of: chrono::NaiveDate,
    ) -> StoreResult<SubscriptionDetails> {
        let db_subscription =
            SubscriptionRow::get_subscription_by_id(conn, &tenant_id, subscription_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

        let subscription: Subscription = db_subscription.try_into()?;

        let schedules: Vec<Schedule> =
            ScheduleRow::list_schedules_by_subscription(conn, &tenant_id, &subscription.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(std::convert::TryInto::try_into)
                .collect::<Result<Vec<_>, Report<_>>>()?;

        // The components and add-ons in force on `as_of`: a change dated later shows from its date.
        let day = as_of;
        let as_of = Some(as_of);
        let subscription_component_rows =
            SubscriptionComponentRow::list_subscription_components_by_subscription(
                conn,
                &tenant_id,
                &subscription.id,
                as_of,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        // Partition by price_id presence for resolution
        let (comp_rows_with_price, comp_rows_without_price): (Vec<_>, Vec<_>) =
            subscription_component_rows
                .into_iter()
                .partition(|row| row.price_id.is_some());

        // Legacy components: deserialize fee JSONB
        let mut subscription_components: Vec<SubscriptionComponent> = comp_rows_without_price
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, Report<_>>>()?;

        let subscription_add_on_rows = SubscriptionAddOnRow::list_by_subscription_id_active(
            conn,
            &tenant_id,
            &subscription.id,
            as_of,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let (addon_rows_with_price, addon_rows_without_price): (Vec<_>, Vec<_>) =
            subscription_add_on_rows
                .into_iter()
                .partition(|row| row.price_id.is_some());

        // Legacy add-ons: deserialize fee JSONB
        let mut subscription_add_ons: Vec<SubscriptionAddOn> = addon_rows_without_price
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, Report<_>>>()?;

        // Resolve new-style components and add-ons from Products + Prices
        if !comp_rows_with_price.is_empty() || !addon_rows_with_price.is_empty() {
            let (prices_by_id, products_by_id) = fetch_prices_and_products(
                conn,
                tenant_id,
                comp_rows_with_price
                    .iter()
                    .filter_map(|r| r.price_id)
                    .chain(addon_rows_with_price.iter().filter_map(|r| r.price_id)),
                comp_rows_with_price
                    .iter()
                    .filter_map(|r| r.product_id)
                    .chain(addon_rows_with_price.iter().filter_map(|r| r.product_id)),
            )
            .await?;
            let units = fetch_price_credit_units(conn, tenant_id, &prices_by_id).await?;

            for row in comp_rows_with_price {
                let resolved = resolve_fee_from_maps(
                    row.price_id,
                    row.product_id,
                    &prices_by_id,
                    &products_by_id,
                    &units,
                );

                let component = if let Some((period, fee)) = resolved {
                    SubscriptionComponent {
                        id: row.id,
                        price_component_id: row.price_component_id,
                        product_id: row.product_id,
                        subscription_id: row.subscription_id,
                        name: row.name,
                        period,
                        fee,
                        price_id: row.price_id,
                        effective_from: row.effective_from,
                        effective_to: row.effective_to,
                        lineage_id: row.lineage_id,
                        added_by_amendment: row.added_by_amendment,
                    }
                } else {
                    row.try_into()?
                };

                subscription_components.push(component);
            }

            for row in addon_rows_with_price {
                let resolved = resolve_fee_from_maps(
                    row.price_id,
                    row.product_id,
                    &prices_by_id,
                    &products_by_id,
                    &units,
                );

                let add_on = if let Some((period, fee)) = resolved {
                    SubscriptionAddOn {
                        id: row.id,
                        subscription_id: row.subscription_id,
                        add_on_id: row.add_on_id,
                        name: row.name,
                        period,
                        fee,
                        created_at: row.created_at,
                        product_id: row.product_id,
                        price_id: row.price_id,
                        quantity: row.quantity,
                        effective_from: row.effective_from,
                        effective_to: row.effective_to,
                        lineage_id: row.lineage_id,
                        added_by_amendment: row.added_by_amendment,
                    }
                } else {
                    row.try_into()?
                };

                subscription_add_ons.push(add_on);
            }
        }

        let mut metric_ids = subscription_components
            .iter()
            .filter_map(SubscriptionComponent::metric_id)
            .collect::<Vec<_>>();

        metric_ids.extend(
            subscription_add_ons
                .iter()
                .filter_map(|sa| sa.fee.metric_id())
                .collect::<Vec<_>>(),
        );

        metric_ids = metric_ids.into_iter().unique().collect::<Vec<_>>();

        let applied_coupons =
            AppliedCouponDetailedRow::list_by_subscription_id(conn, &subscription.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(std::convert::TryInto::try_into)
                .collect::<Result<Vec<_>, Report<_>>>()?;

        let usage_scopes =
            usage_attribution::subscription_usage_scopes(conn, &subscription).await?;
        let billable_metrics: Vec<BillableMetric> =
            BillableMetricRow::get_by_ids(conn, &metric_ids, &subscription.tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .map(|row| {
                    TryInto::<BillableMetric>::try_into(row)
                        .map(|metric| usage_scopes.apply(metric))
                })
                .collect::<Result<Vec<_>, Report<_>>>()?;

        // Look up the linked CheckoutSession to get the checkout URL
        let checkout_url = if subscription.pending_checkout {
            use diesel_models::checkout_sessions::CheckoutSessionRow;

            // Find the active checkout session for this subscription
            let session = CheckoutSessionRow::get_by_subscription(conn, tenant_id, subscription.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            if let Some(session) = session {
                let token = self.checkout_portal_token_tx(conn, &session, false).await?;
                Some(format!(
                    "{}/checkout?token={}",
                    self.settings.public_url, token
                ))
            } else {
                None
            }
        } else {
            None
        };

        let customer: Customer =
            CustomerRow::find_by_id(conn, &subscription.customer_id, &tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .try_into()?;

        let invoicing_entity: InvoicingEntity =
            InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                conn,
                customer.invoicing_entity_id,
                tenant_id,
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into();

        // The trial of the plan version the subscription started on: a plan change ends it. Its
        // freeness is its fact's, recorded at the start (R-100).
        let recorded_free =
            diesel_models::subscription_facts::SubscriptionTrialRow::find(conn, subscription.id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .map(|trial| trial.free);
        let trial_config = {
            let first_switch =
                diesel_models::subscription_events::SubscriptionEventRow::first_plan_switch(
                    conn,
                    subscription.id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
            let trial_version = first_switch
                .map(|(version, _)| version)
                .unwrap_or(subscription.plan_version_id);
            let plan_with_version = PlanRow::get_with_version(conn, trial_version, tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

            if let Some(version) = plan_with_version.version {
                // The subscription's own trial length, else its plan's.
                if let Some(duration_days) = subscription
                    .trial_duration
                    .filter(|days| *days > 0)
                    .and_then(|days| i32::try_from(days).ok())
                    .or(version.trial_duration_days)
                {
                    if duration_days > 0 {
                        // If there's a trialing_plan_id, fetch its name
                        let trialing_plan_name =
                            if let Some(trialing_plan_id) = &version.trialing_plan_id {
                                PlanRow::get_overview_by_id(conn, *trialing_plan_id, tenant_id)
                                    .await
                                    .ok()
                                    .map(|p| p.name)
                            } else {
                                None
                            };

                        let ends_on = subscription
                            .trial_duration
                            .filter(|days| *days > 0)
                            .and_then(|days| {
                                subscription
                                    .billing_start_date
                                    .unwrap_or(subscription.start_date)
                                    .checked_add_signed(chrono::Duration::days(i64::from(days)))
                            })
                            .map(|end| {
                                first_switch.map_or(end, |(_, switched_on)| end.min(switched_on))
                            });
                        Some(TrialConfig {
                            duration_days: duration_days as u32,
                            is_free: recorded_free.unwrap_or(version.trial_is_free),
                            ends_on,
                            trialing_plan_id: version.trialing_plan_id,
                            trialing_plan_name,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        };

        // The changes written ahead and the cancellation still to apply.
        let pending_events = {
            let event_rows =
                SubscriptionChangeRow::list_pending_user_visible(conn, subscription.id, &tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

            let mut events = Vec::new();
            for cancellation in SubscriptionEndingRow::list_pending_cancellations(
                conn,
                tenant_id,
                &[subscription.id],
            )
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            {
                events.push(PendingScheduledEvent {
                    id: common_domain::ids::ScheduledEventId::from(cancellation.id),
                    event_type: crate::domain::enums::ScheduledEventTypeEnum::CancelSubscription,
                    scheduled_date: cancellation.at.date(),
                    new_plan_name: None,
                    new_plan_version_id: None,
                    cancel_reason: cancellation.reason,
                    customer_initiated: cancellation.actor_type
                        == diesel_models::enums::ActorTypeEnum::Customer,
                    amendment_summary: None,
                });
            }
            for row in event_rows {
                let event: ScheduledEvent = row.try_into()?;
                let mut pending = PendingScheduledEvent {
                    id: event.id,
                    event_type: event.event_type.clone(),
                    scheduled_date: event.scheduled_time.date(),
                    new_plan_name: None,
                    new_plan_version_id: None,
                    cancel_reason: None,
                    customer_initiated: event.created_by_customer,
                    amendment_summary: None,
                };

                match &event.event_data {
                    ScheduledEventData::ApplyPlanChange {
                        new_plan_version_id,
                        ..
                    } => {
                        let plan_with_version =
                            PlanRow::get_with_version(conn, *new_plan_version_id, tenant_id)
                                .await
                                .map_err(Into::<Report<StoreError>>::into)?;
                        pending.new_plan_name = Some(plan_with_version.plan.name);
                        pending.new_plan_version_id = Some(*new_plan_version_id);
                    }
                    ScheduledEventData::ApplyAmendment {
                        component_close,
                        component_insert,
                        addon_close,
                        addon_insert,
                    } => {
                        let added_component_names =
                            component_insert.iter().map(|c| c.name.clone()).collect();
                        let added_add_on_names =
                            addon_insert.iter().map(|a| a.name.clone()).collect();

                        // Resolve removed names from the subscription's currently-loaded
                        // components/add-ons (in scope here), avoiding extra DB queries.
                        let removed_component_names = component_close
                            .iter()
                            .filter_map(|id| {
                                subscription_components
                                    .iter()
                                    .find(|c| &c.id == id)
                                    .map(|c| c.name.clone())
                            })
                            .collect();
                        let removed_add_on_names = addon_close
                            .iter()
                            .filter_map(|id| {
                                subscription_add_ons
                                    .iter()
                                    .find(|a| &a.id == id)
                                    .map(|a| a.name.clone())
                            })
                            .collect();

                        pending.amendment_summary = Some(AmendmentSummary {
                            added_component_names,
                            removed_component_names,
                            added_add_on_names,
                            removed_add_on_names,
                        });
                    }
                }

                events.push(pending);
            }
            events.sort_by_key(|event| event.scheduled_date);
            events
        };

        let entitlement_rows = EntitlementRow::list_by_entity(
            conn,
            tenant_id,
            EntitlementEntityId::Subscription(subscription.id),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let entitlements: Vec<Entitlement> = entitlement_rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        let minimum = load_subscription_minimum_on(conn, subscription.id, day).await?;
        let plan_option = plan_option_of(conn, &subscription).await?;

        Ok(SubscriptionDetails {
            subscription,
            price_components: subscription_components,
            add_ons: subscription_add_ons,
            applied_coupons,
            metrics: billable_metrics,
            usage_scopes,
            schedules,
            checkout_url,
            customer,
            invoicing_entity,
            trial_config,
            pending_events,
            entitlements,
            minimum,
            plan_option,
        })
    }
}

/// The plan option `subscription` bought, while its version still has it.
pub(crate) async fn plan_option_of(
    conn: &mut PgConn,
    subscription: &Subscription,
) -> StoreResult<Option<crate::domain::plan_options::PlanOption>> {
    let Some(id) = subscription.plan_option_id else {
        return Ok(None);
    };
    Ok(
        diesel_models::plan_version_options::PlanVersionOptionRow::list_by_plan_version_id(
            conn,
            subscription.tenant_id,
            subscription.plan_version_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .find(|row| row.id == id)
        .map(Into::into),
    )
}

/// The prices `price_ids` name, each as its lineage prices today (R-097), and the products.
pub(crate) async fn fetch_prices_and_products(
    conn: &mut PgConn,
    tenant_id: TenantId,
    price_ids: impl Iterator<Item = PriceId>,
    product_ids: impl Iterator<Item = ProductId>,
) -> StoreResult<(HashMap<PriceId, Price>, HashMap<ProductId, Product>)> {
    fetch_prices_and_products_on(
        conn,
        tenant_id,
        price_ids,
        product_ids,
        common_clock::now().date_naive(),
    )
    .await
}

/// Each lineage root of `roots` with its dated successors, in the order they take over (R-097).
pub(crate) async fn fetch_price_lineages(
    conn: &mut PgConn,
    tenant_id: TenantId,
    roots: &[PriceId],
) -> StoreResult<HashMap<PriceId, Vec<PriceRow>>> {
    let mut lineages: HashMap<PriceId, Vec<PriceRow>> = HashMap::new();
    for row in PriceRow::list_by_ids(conn, roots, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
    {
        lineages.entry(row.id).or_default().insert(0, row);
    }
    for row in PriceRow::successors_of(conn, roots, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
    {
        if let Some(root) = row.supersedes {
            lineages.entry(root).or_default().push(row);
        }
    }
    Ok(lineages)
}

/// The member of a lineage (root first, successors in order) whose rates are in force on `on`.
pub(crate) fn price_in_force(lineage: &[PriceRow], on: NaiveDate) -> Option<&PriceRow> {
    lineage
        .iter()
        .rev()
        .find(|row| row.effective_from.is_none_or(|from| from <= on))
}

/// [`fetch_prices_and_products`] with each price's lineage read on `on`, keyed by the root.
pub(crate) async fn fetch_prices_and_products_on(
    conn: &mut PgConn,
    tenant_id: TenantId,
    price_ids: impl Iterator<Item = PriceId>,
    product_ids: impl Iterator<Item = ProductId>,
    on: NaiveDate,
) -> StoreResult<(HashMap<PriceId, Price>, HashMap<ProductId, Product>)> {
    let price_ids: Vec<PriceId> = price_ids.unique().collect();
    let product_ids: Vec<ProductId> = product_ids.unique().collect();

    let lineages = fetch_price_lineages(conn, tenant_id, &price_ids).await?;
    let mut prices_by_id: HashMap<PriceId, Price> = HashMap::new();
    for (root, lineage) in lineages {
        let Some(row) = price_in_force(&lineage, on) else {
            continue;
        };
        let mut price = Price::try_from(row.clone())?;
        price.id = root;
        prices_by_id.insert(root, price);
    }

    let product_rows = ProductRow::list_by_ids(conn, &product_ids, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let products_by_id: HashMap<ProductId, Product> = product_rows
        .into_iter()
        .map(|row| {
            let id = row.id;
            Product::try_from(row).map(|p| (id, p))
        })
        .collect::<Result<HashMap<_, _>, _>>()?;

    Ok((prices_by_id, products_by_id))
}

/// The credit units the prices are expressed in, or their credit packs grant, by id.
pub(crate) async fn fetch_price_credit_units(
    conn: &mut PgConn,
    tenant_id: TenantId,
    prices_by_id: &HashMap<PriceId, Price>,
) -> StoreResult<CreditUnitsById> {
    load_credit_units(
        conn,
        tenant_id,
        prices_by_id.values().flat_map(Price::credit_unit_ids),
    )
    .await
}

/// Gives a stored fee (a closed component's or add-on's) the credit unit of its price when it was
/// stored without it, so its credits are never billed as money. Fees already carrying their unit,
/// and fees of prices without one, are left as stored.
pub(crate) async fn restore_stored_fee_credit_units(
    conn: &mut PgConn,
    tenant_id: TenantId,
    fees: Vec<(Option<PriceId>, &mut crate::domain::SubscriptionFee)>,
) -> StoreResult<()> {
    use crate::domain::SubscriptionFee;
    let unresolved: Vec<(PriceId, &mut SubscriptionFee)> = fees
        .into_iter()
        .filter_map(|(price_id, fee)| {
            let unresolved = matches!(
                fee,
                SubscriptionFee::Usage {
                    credit_unit: None,
                    ..
                } | SubscriptionFee::Capacity {
                    credit_unit: None,
                    ..
                } | SubscriptionFee::Credit {
                    credit_unit: None,
                    ..
                }
            );
            Some((price_id?, fee)).filter(|_| unresolved)
        })
        .collect();
    if unresolved.is_empty() {
        return Ok(());
    }
    let price_ids: Vec<PriceId> = unresolved.iter().map(|(id, _)| *id).unique().collect();
    let prices: HashMap<PriceId, Price> = PriceRow::list_by_ids(conn, &price_ids, tenant_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .map(|row| Price::try_from(row).map(|price| (price.id, price)))
        .collect::<StoreResult<_>>()?;
    let units = fetch_price_credit_units(conn, tenant_id, &prices).await?;
    let unit = |unit_id: &CreditUnitId| {
        units
            .get(unit_id)
            .ok_or_else(|| Report::new(StoreError::ValueNotFound(format!("credit unit {unit_id}"))))
    };
    for (price_id, fee) in unresolved {
        let Some(price) = prices.get(&price_id) else {
            continue;
        };
        match (fee, &price.credit_unit, &price.pricing) {
            (
                SubscriptionFee::Usage { credit_unit, .. }
                | SubscriptionFee::Capacity { credit_unit, .. },
                Some(priced_in),
                _,
            ) => {
                let unit = unit(&priced_in.unit_id)?;
                *credit_unit = Some(crate::domain::credit_units::FeeCreditUnit {
                    unit_id: unit.id,
                    code: unit.code.as_str().to_owned(),
                    precision: unit.precision,
                    rate: priced_in.rate,
                });
            }
            (
                SubscriptionFee::Credit { credit_unit, .. },
                _,
                crate::domain::prices::Pricing::Credit {
                    credit_unit_id: Some(unit_id),
                    ..
                },
            ) => *credit_unit = Some(unit(unit_id)?.reference()),
            _ => {}
        }
    }
    Ok(())
}

/// `None` when the price, its product or the credit unit it is expressed in is missing.
pub(crate) fn resolve_fee_from_maps(
    price_id: Option<PriceId>,
    product_id: Option<ProductId>,
    prices_by_id: &HashMap<PriceId, Price>,
    products_by_id: &HashMap<ProductId, Product>,
    units: &CreditUnitsById,
) -> Option<(SubscriptionFeeBillingPeriod, crate::domain::SubscriptionFee)> {
    let price = prices_by_id.get(&price_id?)?;
    let product = products_by_id.get(&product_id?)?;
    let fee_structure = &product.fee_structure;
    let fee = resolve_subscription_fee(
        fee_structure,
        &price.pricing,
        price.credit_unit.as_ref(),
        units,
        None,
    )
    .ok()?;
    let period = fee_type_billing_period(fee_structure)
        .unwrap_or_else(|| price.cadence.as_subscription_billing_period());
    Some((period, fee))
}

/// The minimum in force on `on`, from the dated minimums.
pub(crate) async fn load_subscription_minimum_on(
    conn: &mut PgConn,
    subscription_id: common_domain::ids::SubscriptionId,
    on: chrono::NaiveDate,
) -> StoreResult<Option<crate::domain::subscription_minimums::SubscriptionMinimum>> {
    use diesel_models::subscription_adjustments::{
        SubscriptionAdjustmentComponentRow, SubscriptionAdjustmentRow,
    };

    let Some(row) = SubscriptionAdjustmentRow::list_minimum_facts(conn, subscription_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        .into_iter()
        .find(|row| row.effective_from <= on && row.effective_to.is_none_or(|to| on < to))
    else {
        return Ok(None);
    };
    let components = if row.includes_all_components {
        vec![]
    } else {
        SubscriptionAdjustmentComponentRow::list_minimum_fact_scopes(conn, subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .filter(|scope| scope.effective_from == row.effective_from)
            .collect()
    };
    Ok(Some(
        crate::domain::subscription_minimums::SubscriptionMinimum::from_rows(row, components),
    ))
}

pub(crate) async fn load_subscription_minimum(
    conn: &mut PgConn,
    subscription_id: common_domain::ids::SubscriptionId,
) -> StoreResult<Option<crate::domain::subscription_minimums::SubscriptionMinimum>> {
    use diesel_models::subscription_adjustments::{
        SubscriptionAdjustmentComponentRow, SubscriptionAdjustmentRow,
    };

    let Some(row) = SubscriptionAdjustmentRow::find_by_subscription_id(conn, &subscription_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?
    else {
        return Ok(None);
    };

    let components = if row.includes_all_components {
        vec![]
    } else {
        SubscriptionAdjustmentComponentRow::list_by_subscription_id(conn, &subscription_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
    };

    Ok(Some(
        crate::domain::subscription_minimums::SubscriptionMinimum::from_rows(row, components),
    ))
}
