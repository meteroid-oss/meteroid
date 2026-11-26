use crate::StoreResult;
use crate::domain::customer_emails::EmailKind;
use crate::domain::entitlements::Entitlement;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::{
    PaginatedVec, PaginationRequest, Quote, QuoteNew, QuoteWithCustomer, SubscriptionFee,
    enums::QuoteStatusEnum,
    outbox_event::OutboxEvent,
    pgmq::{PgmqQueue, SendEmailRequest},
    quotes::{
        DetailedQuote, QuoteAddOn, QuoteAddOnNew, QuoteCouponNew, QuotePriceComponent,
        QuotePriceComponentNew, QuoteSignature, QuoteSignatureNew,
    },
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::repositories::pgmq::PgmqInterface;
use crate::store::Store;
use common_domain::ids::{
    BaseId, CouponId, CustomerId, EntitlementEntityId, QuoteId, QuotePriceComponentId,
    StoredDocumentId, TenantId,
};
use diesel_models::entitlements::EntitlementRow;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::quote_add_ons::{QuoteAddOnRow, QuoteAddOnRowNew};
use diesel_models::quote_coupons::{QuoteCouponRow, QuoteCouponRowNew};
use diesel_models::quotes::{
    QuoteComponentRow, QuoteComponentRowNew, QuoteRow, QuoteRowNew, QuoteRowUpdate,
    QuoteSignatureRow, QuoteSignatureRowNew,
};
use error_stack::Report;
use scoped_futures::ScopedFutureExt;

/// A quote names a stored, unarchived resource of its customer; returns what the subscription it
/// converts to would be accepted with.
async fn quote_sale_warnings(
    conn: &mut crate::store::PgConn,
    quote: &QuoteNew,
    components: &[QuotePriceComponentNew],
    add_ons: &[QuoteAddOnNew],
) -> StoreResult<crate::domain::SaleWarnings> {
    use crate::repositories::subscriptions::usage_attribution::{
        ensure_resource_for, sale_warnings,
    };
    let resource_type_id = match quote.resource_id {
        Some(resource_id) => Some(
            ensure_resource_for(conn, quote.tenant_id, quote.customer_id, resource_id)
                .await?
                .resource_type_id,
        ),
        None => None,
    };
    let mut metric_ids = components
        .iter()
        .map(|c| &c.fee)
        .chain(add_ons.iter().map(|a| &a.fee))
        .filter_map(SubscriptionFee::metric_id)
        .collect::<Vec<_>>();
    metric_ids.sort_by_key(|id| id.as_uuid());
    metric_ids.dedup();
    sale_warnings(
        conn,
        quote.tenant_id,
        quote.plan_version_id,
        resource_type_id,
        &metric_ids,
    )
    .await
}

/// Quotes may only reference the tenant's own customers, and none that is archived. One query
/// checks both. `archived_error` starts the message for an archived customer.
async fn ensure_quotable_customers(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    customer_ids: &[CustomerId],
    archived_error: &str,
) -> StoreResult<()> {
    use diesel_models::customers::CustomerRow;

    let found = CustomerRow::list_archived_flags_by_ids(conn, tenant_id, customer_ids)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    if let Some(missing) = customer_ids
        .iter()
        .find(|id| !found.iter().any(|(found_id, _, _)| found_id == *id))
    {
        return Err(StoreError::InvalidArgument(format!("Customer not found: {missing}")).into());
    }
    if let Some((id, name, _)) = found.iter().find(|(_, _, archived)| *archived) {
        return Err(StoreError::InvalidArgument(format!("{archived_error}: {name} ({id})")).into());
    }
    Ok(())
}

#[async_trait::async_trait]
pub trait QuotesInterface {
    async fn insert_quote(&self, quote: QuoteNew) -> StoreResult<Quote>;

    async fn insert_quote_batch(&self, quotes: Vec<QuoteNew>) -> StoreResult<Vec<Quote>>;

    async fn get_quote_by_id(&self, tenant_id: TenantId, quote_id: QuoteId) -> StoreResult<Quote>;

    async fn get_quote_with_customer_by_id(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<QuoteWithCustomer>;

    async fn get_detailed_quote_by_id(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<DetailedQuote>;

    async fn list_quotes(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        status: Option<QuoteStatusEnum>,
        search: Option<String>,
        filters: Vec<crate::domain::filters::Filter>,
        order_by: Option<String>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<QuoteWithCustomer>>;

    async fn list_quotes_by_ids(&self, ids: Vec<QuoteId>) -> StoreResult<Vec<Quote>>;

    // async fn update_quote(
    //     &self,
    //     tenant_id: TenantId,
    //     quote_id: QuoteId,
    //     update: QuoteRowUpdate,
    // ) -> StoreResult<Quote>;

    async fn save_quote_documents(
        &self,
        quote_id: QuoteId,
        tenant_id: TenantId,
        pdf_id: StoredDocumentId,
        sharing_key: String,
    ) -> StoreResult<()>;

    async fn accept_quote(
        &self,
        actor: Actor,
        quote_id: QuoteId,
        tenant_id: TenantId,
    ) -> StoreResult<Quote>;

    async fn decline_quote(
        &self,
        actor: Actor,
        quote_id: QuoteId,
        tenant_id: TenantId,
        reason: Option<String>,
    ) -> StoreResult<Quote>;

    async fn publish_quote(
        &self,
        actor: Actor,
        quote_id: QuoteId,
        tenant_id: TenantId,
    ) -> StoreResult<Quote>;

    async fn insert_quote_signature(
        &self,
        actor: Actor,
        signature: QuoteSignatureNew,
        tenant_id: TenantId,
    ) -> StoreResult<QuoteSignature>;

    async fn list_quote_signatures(&self, quote_id: QuoteId) -> StoreResult<Vec<QuoteSignature>>;

    async fn insert_quote_components(
        &self,
        components: Vec<QuotePriceComponentNew>,
    ) -> StoreResult<Vec<QuotePriceComponent>>;

    async fn set_quote_purchase_order(
        &self,
        quote_id: QuoteId,
        tenant_id: TenantId,
        purchase_order: Option<String>,
    ) -> StoreResult<Quote>;

    /// Creates a quote with all its related data (components, add-ons, coupons) in a single transaction.
    /// Add-ons with pending materializations will have their prices created inside the transaction.
    async fn insert_quote_with_details(
        &self,
        quote: QuoteNew,
        components: Vec<QuotePriceComponentNew>,
        add_ons: Vec<QuoteAddOnNew>,
        coupons: Vec<QuoteCouponNew>,
        pending_addon_materializations: Vec<crate::services::PendingMaterialization>,
        actor: Actor,
    ) -> StoreResult<Quote>;

    /// Replaces a quote's fields, components, add-ons, coupons and minimum override.
    /// Entitlements are left untouched. Draft quotes, and pending ones nobody signed yet, can be edited.
    #[allow(clippy::too_many_arguments)]
    async fn replace_quote(
        &self,
        quote: QuoteNew,
        components: Vec<QuotePriceComponentNew>,
        add_ons: Vec<QuoteAddOnNew>,
        coupons: Vec<QuoteCouponNew>,
        pending_addon_materializations: Vec<crate::services::PendingMaterialization>,
        minimum: Option<(i64, crate::domain::plan_minimums::CreateMinimumScope)>,
        actor: Actor,
        bump_revision: bool,
        expected_updated_at: Option<Option<chrono::NaiveDateTime>>,
    ) -> StoreResult<Quote>;

    /// Cancels a quote, preventing future signature.
    /// Only quotes in Draft or Pending status can be cancelled.
    async fn cancel_quote(
        &self,
        actor: Actor,
        quote_id: QuoteId,
        tenant_id: TenantId,
        reason: Option<String>,
    ) -> StoreResult<Quote>;

    /// Sends a quote to its recipients via email, or only to `recipient_email` when given: one of
    /// its recipients or manual countersigners.
    /// This publishes the quote (sets status to Pending if in Draft) and queues the email.
    async fn send_quote(
        &self,
        actor: Actor,
        quote_id: QuoteId,
        tenant_id: TenantId,
        recipient_email: Option<String>,
        custom_message: Option<String>,
    ) -> StoreResult<Quote>;

    /// Look up the quote-level minimum override, if any.
    async fn get_quote_minimum(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Option<crate::domain::QuoteMinimum>>;

    /// Set or replace the quote-level minimum override. Draft-only.
    async fn set_quote_minimum(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        amount_subunit: i64,
        scope: crate::domain::plan_minimums::CreateMinimumScope,
    ) -> StoreResult<crate::domain::QuoteMinimum>;

    /// Remove the quote-level minimum override. Draft-only.
    async fn clear_quote_minimum(&self, tenant_id: TenantId, quote_id: QuoteId) -> StoreResult<()>;
}

#[async_trait::async_trait]
impl QuotesInterface for Store {
    async fn insert_quote(&self, quote: QuoteNew) -> StoreResult<Quote> {
        let mut conn = self.get_conn().await?;

        ensure_quotable_customers(
            &mut conn,
            quote.tenant_id,
            &[quote.customer_id],
            "Cannot create quote for archived customer",
        )
        .await?;

        if let Some(resource_id) = quote.resource_id {
            crate::repositories::subscriptions::usage_attribution::ensure_resource_for(
                &mut conn,
                quote.tenant_id,
                quote.customer_id,
                resource_id,
            )
            .await?;
        }
        let mut quote = quote;
        if quote.custom_properties.is_null() {
            quote.custom_properties = serde_json::json!({});
        }
        let row_new: QuoteRowNew = quote.try_into()?;

        let row = row_new
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        row.try_into()
    }

    async fn insert_quote_batch(&self, quotes: Vec<QuoteNew>) -> StoreResult<Vec<Quote>> {
        let mut conn = self.get_conn().await?;

        use itertools::Itertools;

        let customer_ids: Vec<CustomerId> = quotes.iter().map(|q| q.customer_id).unique().collect();

        if !customer_ids.is_empty() {
            let tenant_id = quotes.first().ok_or(StoreError::InsertError)?.tenant_id;

            ensure_quotable_customers(
                &mut conn,
                tenant_id,
                &customer_ids,
                "Cannot create quote for archived customer",
            )
            .await?;
        }

        let rows_new: Vec<QuoteRowNew> = quotes
            .into_iter()
            .map(|mut q| {
                if q.custom_properties.is_null() {
                    q.custom_properties = serde_json::json!({});
                }
                q
            })
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        let rows = QuoteRowNew::insert_batch(&rows_new, &mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn get_quote_by_id(&self, tenant_id: TenantId, quote_id: QuoteId) -> StoreResult<Quote> {
        let mut conn = self.get_conn().await?;

        QuoteRow::find_by_id(&mut conn, tenant_id, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(std::convert::TryInto::try_into)
    }

    async fn get_quote_with_customer_by_id(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<QuoteWithCustomer> {
        let mut conn = self.get_conn().await?;

        QuoteRow::find_with_customer_by_id(&mut conn, tenant_id, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(std::convert::TryInto::try_into)
    }

    async fn get_detailed_quote_by_id(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<DetailedQuote> {
        let mut conn = self.get_conn().await?;
        detailed_quote_tx(&mut conn, tenant_id, quote_id).await
    }

    async fn list_quotes(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        status: Option<QuoteStatusEnum>,
        search: Option<String>,
        filters: Vec<crate::domain::filters::Filter>,
        order_by: Option<String>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<QuoteWithCustomer>> {
        let mut conn = self.get_conn().await?;

        let cp_filters = if filters.is_empty() {
            vec![]
        } else {
            let definitions = self
                .internal
                .custom_property_definitions
                .load_active(
                    &mut conn,
                    tenant_id,
                    crate::domain::enums::CustomPropertyEntityType::Quote,
                )
                .await?;
            crate::repositories::filter_resolution::resolve_list_filters(
                crate::repositories::filter_resolution::ListEntity::Quote,
                definitions.as_slice(),
                filters,
            )?
        };

        let rows = QuoteRow::list(
            &mut conn,
            tenant_id,
            customer_id,
            status.map(Into::into),
            search,
            cp_filters,
            order_by.as_deref(),
            pagination.into(),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let items: Vec<QuoteWithCustomer> = rows
            .items
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(PaginatedVec {
            items,
            total_pages: rows.total_pages,
            total_results: rows.total_results,
        })
    }

    async fn list_quotes_by_ids(&self, ids: Vec<QuoteId>) -> StoreResult<Vec<Quote>> {
        let mut conn = self.get_conn().await?;

        let rows = QuoteRow::list_by_ids(&mut conn, ids)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    // async fn update_quote(
    //     &self,
    //     tenant_id: TenantId,
    //     quote_id: QuoteId,
    //     update: QuoteRowUpdate,
    // ) -> StoreResult<Quote> {
    //     let mut conn = self.get_conn().await?;
    //
    //     QuoteRow::update_by_id(&mut conn, tenant_id, quote_id, update)
    //         .await
    //         .map_err(Into::<Report<StoreError>>::into)
    //     .and_then(|row| row.try_into())
    // }

    async fn save_quote_documents(
        &self,
        quote_id: QuoteId,
        tenant_id: TenantId,
        pdf_id: StoredDocumentId,
        sharing_key: String,
    ) -> StoreResult<()> {
        let mut conn = self.get_conn().await?;

        QuoteRow::update_documents(&mut conn, quote_id, tenant_id, pdf_id, sharing_key)
            .await
            .map_err(Into::<Report<StoreError>>::into)
    }

    async fn accept_quote(
        &self,
        actor: Actor,
        quote_id: QuoteId,
        tenant_id: TenantId,
    ) -> StoreResult<Quote> {
        self.transaction(|conn| {
            let actor = &actor;
            async move { self.accept_quote_tx(conn, actor, quote_id, tenant_id).await }
                .scope_boxed()
        })
        .await
    }

    async fn decline_quote(
        &self,
        actor: Actor,
        quote_id: QuoteId,
        tenant_id: TenantId,
        reason: Option<String>,
    ) -> StoreResult<Quote> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let now = common_clock::now().naive_utc();

                let update = QuoteRowUpdate {
                    status: Some(diesel_models::enums::QuoteStatusEnum::Declined),

                    declined_at: Some(Some(now)),
                    updated_at: Some(now),
                    decline_reason: Some(reason.clone()),
                    ..Default::default()
                };

                QuoteRow::find_by_id_for_update(conn, tenant_id, quote_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                let updated_row = QuoteRow::update_by_id(conn, tenant_id, quote_id, update)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                self.queue_quote_outcome_emails_tx(conn, tenant_id, &updated_row, reason.clone())
                    .await?;
                Self::append_quote_evidence_tx(
                    conn,
                    tenant_id,
                    &updated_row,
                    crate::domain::quote_signing::QuoteEvidenceNew {
                        kind: crate::domain::quote_signing::QuoteEvidenceKind::Declined,
                        actor_email: match actor {
                            Actor::QuoteRecipient { email } => Some(email.clone()),
                            _ => None,
                        },
                        origin: Default::default(),
                        details: serde_json::json!({ "reason": reason }),
                    },
                )
                .await?;

                let metadata = reason.map(|r| serde_json::json!({ "reason": r }));
                let mut activity = Activity::new(
                    ActivityType::QuoteDeclined,
                    EntityType::Quote,
                    quote_id.as_uuid(),
                );
                if let Some(m) = metadata {
                    activity = activity.with_metadata(m);
                }
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;

                updated_row.try_into()
            }
            .scope_boxed()
        })
        .await
    }

    async fn publish_quote(
        &self,
        actor: Actor,
        quote_id: QuoteId,
        tenant_id: TenantId,
    ) -> StoreResult<Quote> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                let now = common_clock::now().naive_utc();

                let update = QuoteRowUpdate {
                    status: Some(diesel_models::enums::QuoteStatusEnum::Pending),
                    updated_at: Some(now),
                    published_at: Some(Some(now)),
                    ..Default::default()
                };

                let updated_row = QuoteRow::update_by_id(conn, tenant_id, quote_id, update)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                Self::append_quote_evidence_tx(
                    conn,
                    tenant_id,
                    &updated_row,
                    crate::domain::quote_signing::QuoteEvidenceNew {
                        kind: crate::domain::quote_signing::QuoteEvidenceKind::Published,
                        actor_email: None,
                        origin: Default::default(),
                        details: serde_json::json!({}),
                    },
                )
                .await?;

                self.internal
                    .record_audit_tx(
                        conn,
                        tenant_id,
                        actor,
                        AuditInput::Activity(Activity::new(
                            ActivityType::QuotePublished,
                            EntityType::Quote,
                            quote_id.as_uuid(),
                        )),
                    )
                    .await?;

                updated_row.try_into()
            }
            .scope_boxed()
        })
        .await
    }

    async fn insert_quote_signature(
        &self,
        actor: Actor,
        signature: QuoteSignatureNew,
        tenant_id: TenantId,
    ) -> StoreResult<QuoteSignature> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                // Locks out a concurrent edit, which refuses quotes that carry a signature.
                let quote = QuoteRow::find_by_id_for_update(conn, tenant_id, signature.quote_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if quote.status != diesel_models::enums::QuoteStatusEnum::Pending {
                    return Err(StoreError::InvalidArgument(
                        "Only pending quotes can be signed".to_string(),
                    )
                    .into());
                }

                // Signature is forensically binding (electronic signature on a quote)
                // — fold the IP / UA into the activity metadata so the audit row
                // still captures the signing context. Pure auth events live in
                // the dedicated auth audit log; entity_activity has no ip/ua cols.
                let metadata = serde_json::json!({
                    "signed_by_name": signature.signed_by_name,
                    "signed_by_email": signature.signed_by_email,
                    "signed_from_ip": signature.ip_address,
                    "signed_with_ua": signature.user_agent,
                });
                let activity = Activity::new(
                    ActivityType::QuoteSignatureAdded,
                    EntityType::Quote,
                    signature.quote_id.as_uuid(),
                )
                .with_metadata(metadata);
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;

                let signature_row: QuoteSignatureRowNew = signature.into();
                signature_row
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)
                    .map(std::convert::Into::into)
            }
            .scope_boxed()
        })
        .await
    }

    async fn list_quote_signatures(&self, quote_id: QuoteId) -> StoreResult<Vec<QuoteSignature>> {
        let mut conn = self.get_conn().await?;

        QuoteSignatureRow::list_by_quote_id(&mut conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|rows| rows.into_iter().map(std::convert::Into::into).collect())
    }

    async fn insert_quote_components(
        &self,
        components: Vec<QuotePriceComponentNew>,
    ) -> StoreResult<Vec<QuotePriceComponent>> {
        let mut conn = self.get_conn().await?;

        let rows_new: Vec<QuoteComponentRowNew> = components
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        let rows = QuoteComponentRowNew::insert_batch(&rows_new, &mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn set_quote_purchase_order(
        &self,
        quote_id: QuoteId,
        tenant_id: TenantId,
        purchase_order: Option<String>,
    ) -> StoreResult<Quote> {
        let mut conn = self.get_conn().await?;

        QuoteRow::set_purchase_order(&mut conn, quote_id, tenant_id, purchase_order)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(std::convert::TryInto::try_into)
    }

    async fn insert_quote_with_details(
        &self,
        quote: QuoteNew,
        components: Vec<QuotePriceComponentNew>,
        add_ons: Vec<QuoteAddOnNew>,
        coupons: Vec<QuoteCouponNew>,
        pending_addon_materializations: Vec<crate::services::PendingMaterialization>,
        actor: Actor,
    ) -> StoreResult<Quote> {
        self.transaction(|conn| {
            async move {
                ensure_quotable_customers(
                    conn,
                    quote.tenant_id,
                    &[quote.customer_id],
                    "Cannot create quote for archived customer",
                )
                .await?;
                let warnings = quote_sale_warnings(conn, &quote, &components, &add_ons).await?;

                let tenant_id = quote.tenant_id;
                let entitlement_specs = quote.entitlements.clone();

                // Validate user-supplied custom properties against the tenant's quote definitions.
                // A null bag (not managed by the caller) becomes an empty object.
                let mut quote = quote;
                quote.custom_properties = self
                    .internal
                    .validate_and_normalize_custom_properties(
                        conn,
                        tenant_id,
                        crate::domain::enums::CustomPropertyEntityType::Quote,
                        &quote.custom_properties,
                        &serde_json::json!({}),
                        crate::domain::custom_property_definitions::CustomPropertyMode::Replace,
                    )
                    .await?;

                // Insert the quote
                let quote_row: QuoteRowNew = quote.try_into()?;
                let created_quote = quote_row
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let quote_id = created_quote.id;

                insert_quote_children(
                    conn,
                    tenant_id,
                    quote_id,
                    components,
                    add_ons,
                    coupons,
                    &pending_addon_materializations,
                )
                .await?;

                // Insert entitlements if any
                if !entitlement_specs.is_empty() {
                    crate::repositories::entitlements::insert_entitlement_specs(
                        conn,
                        entitlement_specs,
                        EntitlementEntityId::Quote(quote_id),
                        tenant_id,
                    )
                    .await?;
                }

                let customer_id_for_audit = created_quote.customer_id;
                let activity = Activity::new(
                    ActivityType::QuoteCreated,
                    EntityType::Quote,
                    quote_id.as_uuid(),
                )
                .agg_customer(customer_id_for_audit);
                self.internal
                    .record_audit_tx(conn, tenant_id, &actor, AuditInput::Activity(activity))
                    .await?;

                let created: Quote = created_quote.try_into()?;
                Ok(Quote {
                    warnings: warnings.codes,
                    metrics_not_attributed: warnings.metrics_not_attributed,
                    ..created
                })
            }
            .scope_boxed()
        })
        .await
    }

    async fn replace_quote(
        &self,
        quote: QuoteNew,
        components: Vec<QuotePriceComponentNew>,
        add_ons: Vec<QuoteAddOnNew>,
        coupons: Vec<QuoteCouponNew>,
        pending_addon_materializations: Vec<crate::services::PendingMaterialization>,
        minimum: Option<(i64, crate::domain::plan_minimums::CreateMinimumScope)>,
        actor: Actor,
        bump_revision: bool,
        expected_updated_at: Option<Option<chrono::NaiveDateTime>>,
    ) -> StoreResult<Quote> {
        use crate::domain::quote_minimums::QuoteMinimumNew;
        use diesel_models::quote_adjustments::QuoteAdjustmentRow;
        use diesel_models::quotes::QuoteRowReplace;

        if let Some((amount_subunit, scope)) = &minimum {
            validate_quote_minimum(*amount_subunit, scope)?;
        }

        self.transaction(|conn| {
            async move {
                let tenant_id = quote.tenant_id;
                let quote_id = quote.id;

                let existing = QuoteRow::find_by_id_for_update(conn, tenant_id, quote_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                if let Some(expected) = expected_updated_at
                    && expected != existing.updated_at
                {
                    return Err(StoreError::Conflict(
                        "The quote was changed since you opened it".to_string(),
                    )
                    .into());
                }
                match existing.status {
                    diesel_models::enums::QuoteStatusEnum::Draft => {}
                    diesel_models::enums::QuoteStatusEnum::Pending => {
                        let signatures = QuoteSignatureRow::list_by_quote_id(conn, quote_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                        if let Some(signature) = signatures.first() {
                            return Err(StoreError::InvalidArgument(format!(
                                "Quote already signed by {}: cancel it and start a new one instead",
                                signature.signed_by_email
                            ))
                            .into());
                        }
                    }
                    _ => {
                        return Err(StoreError::InvalidArgument(
                            "Only draft or pending quotes can be edited".to_string(),
                        )
                        .into());
                    }
                }

                ensure_quotable_customers(
                    conn,
                    tenant_id,
                    &[quote.customer_id],
                    "Cannot assign quote to archived customer",
                )
                .await?;
                let warnings = quote_sale_warnings(conn, &quote, &components, &add_ons).await?;

                let mut quote = quote;
                quote.custom_properties = self
                    .internal
                    .validate_and_normalize_custom_properties(
                        conn,
                        tenant_id,
                        crate::domain::enums::CustomPropertyEntityType::Quote,
                        &quote.custom_properties,
                        &existing.custom_properties,
                        crate::domain::custom_property_definitions::CustomPropertyMode::Replace,
                    )
                    .await?;

                let row_new: QuoteRowNew = quote.try_into()?;
                let mut updated = QuoteRow::replace_editable(
                    conn,
                    tenant_id,
                    quote_id,
                    QuoteRowReplace::from(row_new),
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

                let revised = bump_revision
                    && existing.status == diesel_models::enums::QuoteStatusEnum::Pending;
                if revised {
                    updated = QuoteRow::update_by_id(
                        conn,
                        tenant_id,
                        quote_id,
                        QuoteRowUpdate {
                            revision: Some(existing.revision + 1),
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                }

                QuoteComponentRow::delete_by_quote_id(conn, quote_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                QuoteAddOnRow::delete_by_quote_id(conn, quote_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;
                QuoteCouponRow::delete_by_quote_id(conn, quote_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                insert_quote_children(
                    conn,
                    tenant_id,
                    quote_id,
                    components,
                    add_ons,
                    coupons,
                    &pending_addon_materializations,
                )
                .await?;

                match minimum {
                    Some((amount_subunit, scope)) => {
                        let new = QuoteMinimumNew {
                            quote_id,
                            amount_subunit,
                            scope,
                        };
                        write_quote_minimum(conn, &new).await?;
                    }
                    None => {
                        QuoteAdjustmentRow::delete_by_quote_id(conn, quote_id)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }
                }

                let activity = if revised {
                    Activity::new(
                        ActivityType::QuoteRevised,
                        EntityType::Quote,
                        quote_id.as_uuid(),
                    )
                    .with_metadata(serde_json::json!({ "revision": updated.revision }))
                } else {
                    Activity::new(
                        ActivityType::QuoteUpdated,
                        EntityType::Quote,
                        quote_id.as_uuid(),
                    )
                }
                .agg_customer(updated.customer_id);
                self.internal
                    .record_audit_tx(conn, tenant_id, &actor, AuditInput::Activity(activity))
                    .await?;

                let document = crate::repositories::quote_signing::refresh_quote_document_tx(
                    conn, tenant_id, quote_id,
                )
                .await?;
                updated.document_hash = Some(document.hash());
                if crate::repositories::quote_approvals::hold_quote_for_approval_tx(
                    conn, tenant_id, quote_id,
                )
                .await?
                {
                    updated.status = diesel_models::enums::QuoteStatusEnum::Draft;
                }
                if revised {
                    Self::append_quote_evidence_tx(
                        conn,
                        tenant_id,
                        &updated,
                        crate::domain::quote_signing::QuoteEvidenceNew {
                            kind: crate::domain::quote_signing::QuoteEvidenceKind::Revised,
                            actor_email: None,
                            origin: Default::default(),
                            details: serde_json::json!({ "revision": updated.revision }),
                        },
                    )
                    .await?;
                }

                let updated: Quote = updated.try_into()?;
                Ok(Quote {
                    warnings: warnings.codes,
                    metrics_not_attributed: warnings.metrics_not_attributed,
                    ..updated
                })
            }
            .scope_boxed()
        })
        .await
    }

    async fn cancel_quote(
        &self,
        actor: Actor,
        quote_id: QuoteId,
        tenant_id: TenantId,
        reason: Option<String>,
    ) -> StoreResult<Quote> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                // First, get the quote to validate its status
                let quote = QuoteRow::find_by_id(conn, tenant_id, quote_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // Only allow cancellation of Draft or Pending quotes
                match quote.status {
                    diesel_models::enums::QuoteStatusEnum::Draft
                    | diesel_models::enums::QuoteStatusEnum::Pending => {}
                    diesel_models::enums::QuoteStatusEnum::Cancelled => {
                        return Err(StoreError::InvalidArgument(
                            "Quote is already cancelled".to_string(),
                        )
                        .into());
                    }
                    diesel_models::enums::QuoteStatusEnum::Accepted => {
                        return Err(StoreError::InvalidArgument(
                            "Cannot cancel an accepted quote".to_string(),
                        )
                        .into());
                    }
                    diesel_models::enums::QuoteStatusEnum::Declined => {
                        return Err(StoreError::InvalidArgument(
                            "Cannot cancel a declined quote".to_string(),
                        )
                        .into());
                    }
                    diesel_models::enums::QuoteStatusEnum::Expired => {
                        return Err(StoreError::InvalidArgument(
                            "Cannot cancel an expired quote".to_string(),
                        )
                        .into());
                    }
                }

                let now = common_clock::now().naive_utc();

                let update = QuoteRowUpdate {
                    status: Some(diesel_models::enums::QuoteStatusEnum::Cancelled),
                    updated_at: Some(now),
                    ..Default::default()
                };

                let updated_row = QuoteRow::update_by_id(conn, tenant_id, quote_id, update)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let metadata = reason.map(|r| serde_json::json!({ "reason": r }));
                let mut activity = Activity::new(
                    ActivityType::QuoteCancelled,
                    EntityType::Quote,
                    quote_id.as_uuid(),
                );
                if let Some(m) = metadata {
                    activity = activity.with_metadata(m);
                }
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;

                updated_row.try_into()
            }
            .scope_boxed()
        })
        .await
    }

    async fn send_quote(
        // TODO rename publish_and_send ?
        &self,
        actor: Actor,
        quote_id: QuoteId,
        tenant_id: TenantId,
        recipient_email: Option<String>,
        custom_message: Option<String>,
    ) -> StoreResult<Quote> {
        self.transaction(|conn| {
            let actor = &actor;
            async move {
                // Get the quote with its details
                let quote = QuoteRow::find_by_id(conn, tenant_id, quote_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // Only allow sending Draft or Pending quotes
                match quote.status {
                    diesel_models::enums::QuoteStatusEnum::Draft => {
                        // Publish the quote (transition to Pending)
                        let now = common_clock::now().naive_utc();
                        let update = QuoteRowUpdate {
                            status: Some(diesel_models::enums::QuoteStatusEnum::Pending),
                            updated_at: Some(now),
                            published_at: Some(Some(now)),
                            ..Default::default()
                        };

                        QuoteRow::update_by_id(conn, tenant_id, quote_id, update)
                            .await
                            .map_err(Into::<Report<StoreError>>::into)?;
                    }
                    diesel_models::enums::QuoteStatusEnum::Pending => {
                        // Already pending, just re-send the email
                    }
                    diesel_models::enums::QuoteStatusEnum::Cancelled => {
                        return Err(StoreError::InvalidArgument(
                            "Cannot send a cancelled quote".to_string(),
                        )
                        .into());
                    }
                    diesel_models::enums::QuoteStatusEnum::Accepted => {
                        return Err(StoreError::InvalidArgument(
                            "Cannot send an already accepted quote".to_string(),
                        )
                        .into());
                    }
                    diesel_models::enums::QuoteStatusEnum::Declined => {
                        return Err(StoreError::InvalidArgument(
                            "Cannot send a declined quote".to_string(),
                        )
                        .into());
                    }
                    diesel_models::enums::QuoteStatusEnum::Expired => {
                        return Err(StoreError::InvalidArgument(
                            "Cannot send an expired quote".to_string(),
                        )
                        .into());
                    }
                }

                // Get the customer to find their invoicing entity
                use diesel_models::customers::CustomerRow;
                let customer = CustomerRow::find_by_id(conn, &quote.customer_id, &tenant_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let email_settings = self
                    .internal
                    .get_email_settings_tx(conn, tenant_id, customer.invoicing_entity_id)
                    .await?;
                if !email_settings.is_enabled(EmailKind::QuoteReady) {
                    return Err(StoreError::InvalidArgument(
                        "The Quote ready email is turned off in Customer emails settings"
                            .to_string(),
                    )
                    .into());
                }

                // Parse recipients from JSON
                let recipients: Vec<crate::domain::quotes::RecipientDetails> =
                    serde_json::from_value(quote.recipients.clone()).map_err(|e| {
                        Report::new(StoreError::InvalidArgument(format!(
                            "Failed to parse recipients: {e}"
                        )))
                    })?;

                if recipients.is_empty() {
                    return Err(StoreError::InvalidArgument(
                        "Quote has no recipients configured".to_string(),
                    )
                    .into());
                }

                // The stored spelling, so the link shares the person's codes and evidence.
                let emailed: Vec<String> = match recipient_email.as_deref().map(str::trim) {
                    None => recipients.iter().map(|r| r.email.clone()).collect(),
                    Some(requested) => {
                        let countersigners: Vec<crate::domain::quotes::QuoteCountersigner> =
                            serde_json::from_value(quote.countersigners.clone()).map_err(|e| {
                                StoreError::SerdeError(
                                    "Failed to deserialize countersigners".to_string(),
                                    e,
                                )
                            })?;
                        let email = recipients
                            .iter()
                            .map(|r| &r.email)
                            .chain(
                                countersigners
                                    .iter()
                                    .filter(|c| !c.automatic)
                                    .map(|c| &c.email),
                            )
                            .find(|email| email.trim().eq_ignore_ascii_case(requested))
                            .cloned()
                            .ok_or_else(|| {
                                StoreError::InvalidArgument(
                                    "Recipient not found in quote recipients".to_string(),
                                )
                            })?;
                        vec![email]
                    }
                };

                let mut email_messages = Vec::new();
                for email in &emailed {
                    email_messages.push(
                        self.quote_ready_email_tx(
                            conn,
                            tenant_id,
                            quote_id,
                            email,
                            custom_message.clone(),
                        )
                        .await?,
                    );
                }

                // Queue all emails
                self.pgmq_send_batch_tx(conn, PgmqQueue::SendEmailRequest, email_messages)
                    .await?;

                let metadata = custom_message
                    .as_ref()
                    .map(|m| serde_json::json!({ "custom_message": m }));
                let mut activity = Activity::new(
                    ActivityType::QuoteSent,
                    EntityType::Quote,
                    quote_id.as_uuid(),
                );
                if let Some(m) = metadata {
                    activity = activity.with_metadata(m);
                }
                self.internal
                    .record_audit_tx(conn, tenant_id, actor, AuditInput::Activity(activity))
                    .await?;

                // Not an edit: `updated_at` guards the editor's copy and stays as it is.
                let updated_quote = QuoteRow::update_by_id(
                    conn,
                    tenant_id,
                    quote_id,
                    QuoteRowUpdate {
                        last_sent_at: Some(Some(common_clock::now().naive_utc())),
                        ..Default::default()
                    },
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;
                Self::append_quote_evidence_tx(
                    conn,
                    tenant_id,
                    &updated_quote,
                    crate::domain::quote_signing::QuoteEvidenceNew {
                        kind: crate::domain::quote_signing::QuoteEvidenceKind::Sent,
                        actor_email: None,
                        origin: Default::default(),
                        details: serde_json::json!({ "recipients": emailed }),
                    },
                )
                .await?;

                updated_quote.try_into()
            }
            .scope_boxed()
        })
        .await
    }

    async fn get_quote_minimum(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
    ) -> StoreResult<Option<crate::domain::QuoteMinimum>> {
        use diesel_models::quote_adjustments::{QuoteAdjustmentComponentRow, QuoteAdjustmentRow};

        let mut conn = self.get_conn().await?;
        // Tenant check.
        QuoteRow::find_by_id(&mut conn, tenant_id, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let Some(row) = QuoteAdjustmentRow::find_by_quote_id(&mut conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        else {
            return Ok(None);
        };
        let components = if row.includes_all_components {
            vec![]
        } else {
            QuoteAdjustmentComponentRow::list_by_quote_id(&mut conn, quote_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        };
        Ok(Some(crate::domain::QuoteMinimum::from_rows(
            row, components,
        )))
    }

    async fn set_quote_minimum(
        &self,
        tenant_id: TenantId,
        quote_id: QuoteId,
        amount_subunit: i64,
        scope: crate::domain::plan_minimums::CreateMinimumScope,
    ) -> StoreResult<crate::domain::QuoteMinimum> {
        use crate::domain::quote_minimums::QuoteMinimumNew;
        use diesel_models::quote_adjustments::{QuoteAdjustmentComponentRow, QuoteAdjustmentRow};

        validate_quote_minimum(amount_subunit, &scope)?;

        let mut conn = self.get_conn().await?;
        let quote = QuoteRow::find_by_id(&mut conn, tenant_id, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if quote.status != diesel_models::enums::QuoteStatusEnum::Draft {
            return Err(Report::new(StoreError::InvalidArgument(
                "Cannot edit minimum on a non-draft quote".to_string(),
            )));
        }

        let new = QuoteMinimumNew {
            quote_id,
            amount_subunit,
            scope: scope.clone(),
        };

        self.transaction_with(&mut conn, |conn| {
            let new = new.clone();
            async move { write_quote_minimum(conn, &new).await }.scope_boxed()
        })
        .await?;

        let row = QuoteAdjustmentRow::find_by_quote_id(&mut conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .ok_or_else(|| {
                Report::new(StoreError::ValueNotFound(
                    "quote_minimum row vanished after upsert".to_string(),
                ))
            })?;
        let components = if row.includes_all_components {
            vec![]
        } else {
            QuoteAdjustmentComponentRow::list_by_quote_id(&mut conn, quote_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
        };
        Ok(crate::domain::QuoteMinimum::from_rows(row, components))
    }

    async fn clear_quote_minimum(&self, tenant_id: TenantId, quote_id: QuoteId) -> StoreResult<()> {
        use diesel_models::quote_adjustments::QuoteAdjustmentRow;

        let mut conn = self.get_conn().await?;
        let quote = QuoteRow::find_by_id(&mut conn, tenant_id, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if quote.status != diesel_models::enums::QuoteStatusEnum::Draft {
            return Err(Report::new(StoreError::InvalidArgument(
                "Cannot remove minimum on a non-draft quote".to_string(),
            )));
        }
        QuoteAdjustmentRow::delete_by_quote_id(&mut conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        Ok(())
    }
}

async fn insert_quote_children(
    conn: &mut diesel_models::PgConn,
    tenant_id: TenantId,
    quote_id: QuoteId,
    components: Vec<QuotePriceComponentNew>,
    add_ons: Vec<QuoteAddOnNew>,
    coupons: Vec<QuoteCouponNew>,
    pending_addon_materializations: &[crate::services::PendingMaterialization],
) -> StoreResult<()> {
    // A seat add-on is held once by the subscription the quote makes (R-138).
    crate::services::subscriptions::utils::ensure_seat_fees_once(
        add_ons
            .iter()
            .map(|ao| (ao.add_on_id, ao.name.as_str(), &ao.fee, ao.quantity)),
    )?;
    // Insert components if any
    if !components.is_empty() {
        let component_rows: Vec<QuoteComponentRowNew> = components
            .into_iter()
            .map(|c| QuoteComponentRowNew {
                id: QuotePriceComponentId::new(),
                quote_id,
                name: c.name,
                price_component_id: c.price_component_id,
                product_id: c.product_id,
                period: c.period.into(),
                legacy_fee: Some(serde_json::to_value(&c.fee).unwrap_or_default()),
                is_override: c.is_override,
                price_id: c.price_id,
                example_usage_quantity: c.example_usage_quantity,
                description: c.description,
                sort_order: c.sort_order,
                discount_percent: c.discount_percent,
                list_fee: c
                    .list_fee
                    .as_ref()
                    .map(|f| serde_json::to_value(f).unwrap_or_default()),
            })
            .collect();

        QuoteComponentRowNew::insert_batch(&component_rows, conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }

    // Materialize add-on prices inside the transaction
    let mut materialized_addon_prices: std::collections::HashMap<
        usize,
        common_domain::ids::PriceId,
    > = std::collections::HashMap::new();
    for mat in pending_addon_materializations {
        use crate::domain::price_components::PriceComponentNewInternal;
        use crate::repositories::price_components::resolve_component_internal;

        let internal = PriceComponentNewInternal {
            name: mat.name.clone(),
            product_ref: mat.product_ref.clone(),
            fee_structure: mat.fee_structure.clone(),
            prices: vec![mat.price_entry.clone()],
        };
        let charge = resolve_component_internal(
            conn,
            &internal,
            tenant_id,
            mat.product_family_id,
            &mat.currency,
            false,
        )
        .await?;
        if let Some(price_id) = charge.price_ids.into_iter().next() {
            materialized_addon_prices.insert(mat.component_index, price_id);
        }
    }

    // Insert add-ons if any, patching materialized prices
    if !add_ons.is_empty() {
        let add_on_rows: Vec<QuoteAddOnRowNew> = add_ons
            .into_iter()
            .enumerate()
            .map(|(idx, ao)| {
                let mut row: QuoteAddOnRowNew = ao.try_into()?;
                if let Some(price_id) = materialized_addon_prices.get(&idx) {
                    row.price_id = Some(*price_id);
                    row.legacy_fee = None;
                }
                Ok(row)
            })
            .collect::<Result<Vec<_>, StoreErrorReport>>()?;

        QuoteAddOnRowNew::insert_batch(&add_on_rows, conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }

    // Insert coupons if any
    if !coupons.is_empty() {
        let coupon_rows: Vec<QuoteCouponRowNew> =
            coupons.into_iter().map(std::convert::Into::into).collect();

        QuoteCouponRowNew::insert_batch(&coupon_rows, conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }

    Ok(())
}

fn validate_quote_minimum(
    amount_subunit: i64,
    scope: &crate::domain::plan_minimums::CreateMinimumScope,
) -> StoreResult<()> {
    use crate::domain::plan_minimums::CreateMinimumScope;

    if amount_subunit <= 0 {
        return Err(Report::new(StoreError::InvalidArgument(
            "Minimum amount_subunit must be > 0".to_string(),
        )));
    }
    if let CreateMinimumScope::Products {
        product_ids,
        extra_product_names,
    } = scope
        && product_ids.is_empty()
        && extra_product_names.is_empty()
    {
        return Err(Report::new(StoreError::InvalidArgument(
            "Scope=Products requires at least one product or extra".to_string(),
        )));
    }
    Ok(())
}

async fn write_quote_minimum(
    conn: &mut diesel_models::PgConn,
    minimum: &crate::domain::quote_minimums::QuoteMinimumNew,
) -> StoreResult<()> {
    use diesel_models::quote_adjustments::QuoteAdjustmentComponentRow;

    minimum
        .to_row()
        .upsert(conn)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    QuoteAdjustmentComponentRow::delete_by_quote_id(conn, minimum.quote_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;
    let comps = minimum.component_rows();
    if !comps.is_empty() {
        QuoteAdjustmentComponentRow::insert_batch(conn, &comps)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
    }
    Ok(())
}

impl Store {
    pub(crate) async fn accept_quote_tx(
        &self,
        conn: &mut crate::store::PgConn,
        actor: &Actor,
        quote_id: QuoteId,
        tenant_id: TenantId,
    ) -> StoreResult<Quote> {
        let now = common_clock::now().naive_utc();

        // Its subscription could never be created for a resource sold meanwhile (R-145).
        let quote = QuoteRow::find_by_id(conn, tenant_id, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        if let Some(resource_id) = quote.resource_id {
            use crate::repositories::subscriptions::usage_attribution as attribution;
            attribution::ensure_resource_for(conn, tenant_id, quote.customer_id, resource_id)
                .await?;
            attribution::ensure_resource_unsold(
                conn,
                tenant_id,
                resource_id,
                quote.billing_start_date.unwrap_or(now.date()),
                quote.billing_end_date,
                None,
            )
            .await?;
        }

        // Update quote status
        let update = QuoteRowUpdate {
            status: Some(diesel_models::enums::QuoteStatusEnum::Accepted),
            trial_duration_days: None,
            billing_start_date: None,
            billing_end_date: None,
            billing_day_anchor: None,
            accepted_at: Some(Some(now)),
            updated_at: Some(now),
            valid_until: None,
            expires_at: None,
            declined_at: None,
            internal_notes: None,
            cover_image: None,
            overview: None,
            terms_and_services: None,
            net_terms: None,
            attachments: None,
            pdf_document_id: None,
            sharing_key: None,
            converted_to_invoice_id: None,
            converted_to_subscription_id: None,
            converted_at: None,
            recipients: None,
            activation_condition: None,
            auto_advance_invoices: None,
            charge_automatically: None,
            invoice_memo: None,
            invoice_threshold: None,
            create_subscription_on_acceptance: None,
            payment_methods_config: None,
            ..Default::default()
        };

        let updated_row = QuoteRow::update_by_id(conn, tenant_id, quote_id, update)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        self.queue_quote_outcome_emails_tx(conn, tenant_id, &updated_row, None)
            .await?;

        let should_create_subscription = updated_row.create_subscription_on_acceptance;
        let quote: Quote = updated_row.try_into()?;

        if should_create_subscription {
            // Outbox path also records the audit row (quote.accepted).
            self.internal
                .record_outbox_batch_tx(
                    conn,
                    tenant_id,
                    actor,
                    vec![OutboxEvent::quote_accepted(quote.clone().into())],
                )
                .await?;
        } else {
            // No outbox event in this branch; write the audit row directly.
            self.internal
                .record_audit_tx(
                    conn,
                    tenant_id,
                    actor,
                    AuditInput::Activity(Activity::new(
                        ActivityType::QuoteAccepted,
                        EntityType::Quote,
                        quote_id.as_uuid(),
                    )),
                )
                .await?;
        }

        Ok(quote)
    }

    /// What a quote's outcome emails, in the transaction that sets it: on acceptance, each
    /// customer recipient with their own portal link; accepted or declined, the quote's owner.
    async fn queue_quote_outcome_emails_tx(
        &self,
        conn: &mut crate::store::PgConn,
        tenant_id: TenantId,
        quote: &QuoteRow,
        reason: Option<String>,
    ) -> StoreResult<()> {
        use diesel_models::customers::CustomerRow;
        use diesel_models::users::UserRow;

        let accepted = match quote.status {
            diesel_models::enums::QuoteStatusEnum::Accepted => true,
            diesel_models::enums::QuoteStatusEnum::Declined => false,
            _ => return Ok(()),
        };
        let customer = CustomerRow::find_by_id(conn, &quote.customer_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let mut messages: Vec<crate::domain::pgmq::PgmqMessageNew> = vec![];

        if accepted {
            let recipients: Vec<crate::domain::quotes::RecipientDetails> =
                serde_json::from_value(quote.recipients.clone()).map_err(|e| {
                    StoreError::SerdeError("Failed to deserialize recipients".to_string(), e)
                })?;
            let accepted_on = quote
                .accepted_at
                .unwrap_or_else(|| common_clock::now().naive_utc())
                .date();
            for recipient in recipients {
                let token = self
                    .quote_portal_token_tx(conn, tenant_id, quote, recipient.email.clone(), false)
                    .await?;
                messages.push(
                    SendEmailRequest::QuoteAccepted {
                        tenant_id,
                        quote_id: quote.id,
                        invoicing_entity_id: customer.invoicing_entity_id,
                        quote_number: quote.quote_number.clone(),
                        accepted_on,
                        recipient_email: recipient.email,
                        portal_url: format!(
                            "{}/portal/quote?token={}",
                            &self.settings.public_url, token
                        ),
                    }
                    .try_into()?,
                );
            }
        }

        // A quote created through the API has no owner: nobody on the team to tell.
        if let Some(owner_id) = quote.owner_id {
            let owner = UserRow::find_members_by_ids(conn, &[owner_id], tenant_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                .into_iter()
                .next();
            if let Some(owner) = owner {
                messages.push(
                    SendEmailRequest::QuoteOutcome {
                        tenant_id,
                        quote_id: quote.id,
                        invoicing_entity_id: customer.invoicing_entity_id,
                        quote_number: quote.quote_number.clone(),
                        customer_name: customer.name.clone(),
                        accepted,
                        reason,
                        recipient_email: owner.email,
                    }
                    .try_into()?,
                );
            }
        }

        if messages.is_empty() {
            return Ok(());
        }
        self.pgmq_send_batch_tx(conn, PgmqQueue::SendEmailRequest, messages)
            .await
    }

    /// One recipient's "quote ready" email, carrying their own portal link.
    pub(crate) async fn quote_ready_email_tx(
        &self,
        conn: &mut crate::store::PgConn,
        tenant_id: TenantId,
        quote_id: QuoteId,
        recipient_email: &str,
        custom_message: Option<String>,
    ) -> StoreResult<crate::domain::pgmq::PgmqMessageNew> {
        use diesel_models::customers::CustomerRow;

        let quote = QuoteRow::find_by_id(conn, tenant_id, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let customer = CustomerRow::find_by_id(conn, &quote.customer_id, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;
        let invoicing_entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
            conn,
            customer.invoicing_entity_id,
            tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let token = self
            .quote_portal_token_tx(conn, tenant_id, &quote, recipient_email.to_string(), false)
            .await?;
        let portal_url = format!("{}/portal/quote?token={}", &self.settings.public_url, token);

        SendEmailRequest::QuoteReady {
            tenant_id,
            quote_id,
            invoicing_entity_id: customer.invoicing_entity_id,
            quote_number: quote.quote_number.clone(),
            expires_at: quote.expires_at.map(|dt| dt.date()),
            company_name: invoicing_entity.legal_name.clone(),
            logo_attachment_id: invoicing_entity.logo_attachment_id,
            recipient_emails: vec![recipient_email.to_string()],
            portal_url,
            custom_message,
            currency: quote.currency.clone(),
        }
        .try_into()
    }
}

pub(crate) async fn detailed_quote_tx(
    conn: &mut crate::store::PgConn,
    tenant_id: TenantId,
    quote_id: QuoteId,
) -> StoreResult<DetailedQuote> {
    // Get quote with customer
    let quote_with_customer: QuoteWithCustomer =
        QuoteRow::find_with_customer_by_id(conn, tenant_id, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(std::convert::TryInto::try_into)?;

    let component_rows = QuoteComponentRow::list_by_quote_id(conn, quote_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    let add_on_rows = QuoteAddOnRow::list_by_quote_id(conn, quote_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

    // Partition components and add-ons by price_id presence for v2 resolution
    let (comp_rows_with_price, comp_rows_without_price): (Vec<_>, Vec<_>) = component_rows
        .into_iter()
        .partition(|row| row.price_id.is_some());

    let (addon_rows_with_price, addon_rows_without_price): (Vec<_>, Vec<_>) = add_on_rows
        .into_iter()
        .partition(|row| row.price_id.is_some());

    // Legacy rows: deserialize fee from JSONB
    let mut components: Vec<QuotePriceComponent> = comp_rows_without_price
        .into_iter()
        .map(std::convert::TryInto::try_into)
        .collect::<Result<Vec<_>, _>>()?;

    let mut add_ons: Vec<QuoteAddOn> = addon_rows_without_price
        .into_iter()
        .map(std::convert::TryInto::try_into)
        .collect::<Result<Vec<_>, _>>()?;

    // Resolve v2 rows from their charges' structures + prices
    if !comp_rows_with_price.is_empty() || !addon_rows_with_price.is_empty() {
        use crate::repositories::subscriptions::{
            ChargeShapes, fetch_price_credit_units, fetch_prices, resolve_fee_from_maps,
        };

        let prices_by_id = fetch_prices(
            conn,
            tenant_id,
            comp_rows_with_price
                .iter()
                .filter_map(|r| r.price_id)
                .chain(addon_rows_with_price.iter().filter_map(|r| r.price_id)),
        )
        .await?;
        let component_ids: Vec<_> = comp_rows_with_price
            .iter()
            .filter_map(|r| r.price_component_id)
            .collect();
        let add_on_ids: Vec<_> = addon_rows_with_price.iter().map(|r| r.add_on_id).collect();
        let shapes = ChargeShapes::load_ids(conn, tenant_id, &component_ids, &add_on_ids).await?;
        let units = fetch_price_credit_units(conn, tenant_id, &prices_by_id).await?;

        for row in comp_rows_with_price {
            let shape = shapes.of_charge(row.price_component_id, None, row.legacy_fee.as_ref());
            let resolved =
                resolve_fee_from_maps(row.price_id, shape.as_ref(), &prices_by_id, &units);

            let component = if let Some((period, mut fee)) = resolved {
                if let Some(snapshot) = row
                    .legacy_fee
                    .as_ref()
                    .and_then(|v| SubscriptionFee::try_from(v).ok())
                {
                    fee.keep_slots_from(&snapshot);
                }
                QuotePriceComponent {
                    id: row.id,
                    name: row.name,
                    quote_id: row.quote_id,
                    price_component_id: row.price_component_id,
                    product_id: row.product_id,
                    period,
                    fee,
                    is_override: row.is_override,
                    price_id: row.price_id,
                    example_usage_quantity: row.example_usage_quantity,
                    description: row.description,
                    sort_order: row.sort_order,
                    discount_percent: row.discount_percent,
                    list_fee: row.list_fee.map(TryInto::try_into).transpose()?,
                }
            } else {
                row.try_into()?
            };

            components.push(component);
        }

        for row in addon_rows_with_price {
            let shape = shapes.of_charge(None, Some(row.add_on_id), row.legacy_fee.as_ref());
            let resolved =
                resolve_fee_from_maps(row.price_id, shape.as_ref(), &prices_by_id, &units);

            let add_on = if let Some((period, fee)) = resolved {
                QuoteAddOn {
                    id: row.id,
                    name: row.name,
                    quote_id: row.quote_id,
                    add_on_id: row.add_on_id,
                    period,
                    fee,
                    product_id: row.product_id,
                    price_id: row.price_id,
                    quantity: row.quantity,
                    description: row.description,
                    sort_order: row.sort_order,
                    discount_percent: row.discount_percent,
                    list_fee: row.list_fee.map(TryInto::try_into).transpose()?,
                }
            } else {
                row.try_into()?
            };

            add_ons.push(add_on);
        }
    }

    components.sort_by_key(|c| c.sort_order);
    add_ons.sort_by_key(|a| a.sort_order);

    let signatures = QuoteSignatureRow::list_by_quote_id(conn, quote_id)
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .map(|l| l.into_iter().map(std::convert::Into::into).collect())?;

    let invoicing_entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
        conn,
        quote_with_customer.customer.invoicing_entity_id,
        tenant_id,
    )
    .await
    .map_err(Into::<Report<StoreError>>::into)
    .map(std::convert::Into::into)?;

    let coupons: Vec<crate::domain::quotes::QuoteCoupon> =
        QuoteCouponRow::list_by_quote_id(conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|l| l.into_iter().map(std::convert::Into::into).collect())?;

    let coupon_ids: Vec<CouponId> = coupons.iter().map(|c| c.coupon_id).collect();
    let coupon_details: Vec<crate::domain::coupons::Coupon> = if coupon_ids.is_empty() {
        vec![]
    } else {
        diesel_models::coupons::CouponRow::list_by_ids(conn, &coupon_ids, &tenant_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<_, _>>()?
    };

    let entitlement_rows =
        EntitlementRow::list_by_entity(conn, tenant_id, EntitlementEntityId::Quote(quote_id))
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

    let entitlements: Vec<Entitlement> = entitlement_rows
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<Vec<_>, _>>()?;

    let minimum = {
        use diesel_models::quote_adjustments::{QuoteAdjustmentComponentRow, QuoteAdjustmentRow};
        match QuoteAdjustmentRow::find_by_quote_id(conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?
        {
            Some(row) => {
                let components = if row.includes_all_components {
                    vec![]
                } else {
                    QuoteAdjustmentComponentRow::list_by_quote_id(conn, quote_id)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?
                };
                Some(crate::domain::QuoteMinimum::from_rows(row, components))
            }
            None => None,
        }
    };

    Ok(DetailedQuote {
        quote: quote_with_customer.quote,
        customer: quote_with_customer.customer,
        invoicing_entity,
        components,
        add_ons,
        coupons,
        coupon_details,
        signatures,
        entitlements,
        minimum,
    })
}
