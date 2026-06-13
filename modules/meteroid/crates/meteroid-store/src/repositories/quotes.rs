use crate::StoreResult;
use crate::domain::entitlements::Entitlement;
use crate::domain::entity_activity::{Activity, ActivityType, Actor, AuditInput, EntityType};
use crate::domain::{
    PaginatedVec, PaginationRequest, Quote, QuoteNew, QuoteWithCustomer,
    enums::QuoteStatusEnum,
    outbox_event::OutboxEvent,
    pgmq::{PgmqQueue, SendEmailRequest},
    quotes::{
        DetailedQuote, QuoteAddOn, QuoteAddOnNew, QuoteCouponNew, QuotePriceComponent,
        QuotePriceComponentNew, QuoteSignature, QuoteSignatureNew,
    },
};
use crate::errors::{StoreError, StoreErrorReport};
use crate::jwt_claims::{ResourceAccess, generate_portal_token};
use crate::repositories::pgmq::PgmqInterface;
use crate::store::Store;
use common_domain::ids::{
    BaseId, CustomerId, EntitlementEntityId, QuoteId, QuotePriceComponentId, StoredDocumentId,
    TenantId, UserId,
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
        created_by: uuid::Uuid,
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

    /// Sends a quote to its recipients via email.
    /// This publishes the quote (sets status to Pending if in Draft) and queues the email.
    async fn send_quote(
        &self,
        actor: Actor,
        quote_id: QuoteId,
        tenant_id: TenantId,
        custom_message: Option<String>,
    ) -> StoreResult<Quote>;
}

#[async_trait::async_trait]
impl QuotesInterface for Store {
    async fn insert_quote(&self, quote: QuoteNew) -> StoreResult<Quote> {
        let mut conn = self.get_conn().await?;

        // Check if customer is archived before creating quote (efficient query)
        use diesel_models::customers::CustomerRow;

        if let Some((id, name)) = CustomerRow::find_archived_customer_in_batch(
            &mut conn,
            quote.tenant_id,
            vec![quote.customer_id],
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?
        {
            return Err(StoreError::InvalidArgument(format!(
                "Cannot create quote for archived customer: {} ({})",
                name, id
            ))
            .into());
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

        // Check if any customers are archived before creating quotes (efficient query)
        use diesel_models::customers::CustomerRow;
        use itertools::Itertools;

        let customer_ids: Vec<CustomerId> = quotes.iter().map(|q| q.customer_id).unique().collect();

        if !customer_ids.is_empty() {
            let tenant_id = quotes.first().ok_or(StoreError::InsertError)?.tenant_id;

            if let Some((id, name)) =
                CustomerRow::find_archived_customer_in_batch(&mut conn, tenant_id, customer_ids)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?
            {
                return Err(StoreError::InvalidArgument(format!(
                    "Cannot create quote for archived customer: {} ({})",
                    name, id
                ))
                .into());
            }
        }

        let rows_new: Vec<QuoteRowNew> = quotes
            .into_iter()
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

        // Get quote with customer
        let quote_with_customer: QuoteWithCustomer =
            QuoteRow::find_with_customer_by_id(&mut conn, tenant_id, quote_id)
                .await
                .map_err(Into::<Report<StoreError>>::into)
                .and_then(std::convert::TryInto::try_into)?;

        let component_rows = QuoteComponentRow::list_by_quote_id(&mut conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        let add_on_rows = QuoteAddOnRow::list_by_quote_id(&mut conn, quote_id)
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

        // Resolve v2 rows from Products + Prices
        if !comp_rows_with_price.is_empty() || !addon_rows_with_price.is_empty() {
            use crate::repositories::subscriptions::{
                fetch_prices_and_products, resolve_fee_from_maps,
            };

            let (prices_by_id, products_by_id) = fetch_prices_and_products(
                &mut conn,
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

            for row in comp_rows_with_price {
                let resolved = resolve_fee_from_maps(
                    row.price_id,
                    row.product_id,
                    &prices_by_id,
                    &products_by_id,
                );

                let component = if let Some((period, fee)) = resolved {
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
                    }
                } else {
                    row.try_into()?
                };

                components.push(component);
            }

            for row in addon_rows_with_price {
                let resolved = resolve_fee_from_maps(
                    row.price_id,
                    row.product_id,
                    &prices_by_id,
                    &products_by_id,
                );

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
                    }
                } else {
                    row.try_into()?
                };

                add_ons.push(add_on);
            }
        }

        let signatures = QuoteSignatureRow::list_by_quote_id(&mut conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|l| l.into_iter().map(std::convert::Into::into).collect())?;

        let invoicing_entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
            &mut conn,
            quote_with_customer.customer.invoicing_entity_id,
            tenant_id,
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)
        .map(std::convert::Into::into)?;

        let coupons = QuoteCouponRow::list_by_quote_id(&mut conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|l| l.into_iter().map(std::convert::Into::into).collect())?;

        let entitlement_rows = EntitlementRow::list_by_entity(
            &mut conn,
            tenant_id,
            EntitlementEntityId::Quote(quote_id),
        )
        .await
        .map_err(Into::<Report<StoreError>>::into)?;

        let entitlements: Vec<Entitlement> = entitlement_rows
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(DetailedQuote {
            quote: quote_with_customer.quote,
            customer: quote_with_customer.customer,
            invoicing_entity,
            components,
            add_ons,
            coupons,
            signatures,
            entitlements,
        })
    }

    async fn list_quotes(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        status: Option<QuoteStatusEnum>,
        search: Option<String>,
        order_by: Option<String>,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<QuoteWithCustomer>> {
        let mut conn = self.get_conn().await?;

        let rows = QuoteRow::list(
            &mut conn,
            tenant_id,
            customer_id,
            status.map(Into::into),
            search,
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
            async move {
                let now = chrono::Utc::now().naive_utc();

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
                };

                let updated_row = QuoteRow::update_by_id(conn, tenant_id, quote_id, update)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

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

                Ok::<Quote, Report<StoreError>>(quote)
            }
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
                let now = chrono::Utc::now().naive_utc();

                let update = QuoteRowUpdate {
                    status: Some(diesel_models::enums::QuoteStatusEnum::Declined),

                    declined_at: Some(Some(now)),
                    updated_at: Some(now),
                    ..Default::default()
                };

                let updated_row = QuoteRow::update_by_id(conn, tenant_id, quote_id, update)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

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
                let now = chrono::Utc::now().naive_utc();

                let update = QuoteRowUpdate {
                    status: Some(diesel_models::enums::QuoteStatusEnum::Pending),
                    updated_at: Some(now),
                    ..Default::default()
                };

                let updated_row = QuoteRow::update_by_id(conn, tenant_id, quote_id, update)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

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
        created_by: uuid::Uuid,
    ) -> StoreResult<Quote> {
        use diesel_models::customers::CustomerRow;

        self.transaction(|conn| {
            async move {
                // Check if customer is archived before creating quote
                let customer_ids = vec![quote.customer_id];
                if let Some((id, name)) = CustomerRow::find_archived_customer_in_batch(
                    conn,
                    quote.tenant_id,
                    customer_ids,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?
                {
                    return Err(StoreError::InvalidArgument(format!(
                        "Cannot create quote for archived customer: {} ({})",
                        name, id
                    ))
                    .into());
                }

                let tenant_id = quote.tenant_id;
                let entitlement_specs = quote.entitlements.clone();

                // Insert the quote
                let quote_row: QuoteRowNew = quote.try_into()?;
                let created_quote = quote_row
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                let quote_id = created_quote.id;

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
                for mat in &pending_addon_materializations {
                    use crate::domain::price_components::PriceComponentNewInternal;
                    use crate::repositories::price_components::resolve_component_internal;

                    let internal = PriceComponentNewInternal {
                        name: mat.name.clone(),
                        product_ref: mat.product_ref.clone(),
                        prices: vec![mat.price_entry.clone()],
                    };
                    let (_product_id, price_ids) = resolve_component_internal(
                        conn,
                        &internal,
                        tenant_id,
                        mat.product_family_id,
                        &mat.currency,
                        false,
                    )
                    .await?;
                    if let Some(price_id) = price_ids.into_iter().next() {
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
                    .record_audit_tx(
                        conn,
                        tenant_id,
                        &Actor::User {
                            id: UserId::from(created_by),
                        },
                        AuditInput::Activity(activity),
                    )
                    .await?;

                created_quote.try_into()
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

                let now = chrono::Utc::now().naive_utc();

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
                        let now = chrono::Utc::now().naive_utc();
                        let update = QuoteRowUpdate {
                            status: Some(diesel_models::enums::QuoteStatusEnum::Pending),
                            updated_at: Some(now),
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

                // Get the invoicing entity details
                let invoicing_entity = InvoicingEntityRow::get_invoicing_entity_by_id_and_tenant(
                    conn,
                    customer.invoicing_entity_id,
                    tenant_id,
                )
                .await
                .map_err(Into::<Report<StoreError>>::into)?;

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

                // Generate one email request per recipient, each with their own JWT token
                let mut email_messages = Vec::new();
                for recipient in &recipients {
                    // Generate a unique JWT token for this recipient
                    let token = generate_portal_token(
                        &self.settings.jwt_secret,
                        tenant_id,
                        ResourceAccess::Quote {
                            quote_id,
                            recipient_email: recipient.email.clone(),
                        },
                    )?;

                    let portal_url =
                        format!("{}/portal/quote?token={}", &self.settings.public_url, token);

                    let email_request = SendEmailRequest::QuoteReady {
                        tenant_id,
                        quote_id,
                        invoicing_entity_id: customer.invoicing_entity_id,
                        quote_number: quote.quote_number.clone(),
                        expires_at: quote.expires_at.map(|dt| dt.date()),
                        company_name: invoicing_entity.legal_name.clone(),
                        logo_attachment_id: invoicing_entity.logo_attachment_id,
                        recipient_emails: vec![recipient.email.clone()],
                        portal_url,
                        custom_message: custom_message.clone(),
                        currency: quote.currency.clone(),
                    };

                    // Convert to PgmqMessageNew
                    let message: crate::domain::pgmq::PgmqMessageNew = email_request.try_into()?;
                    email_messages.push(message);
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

                // Return the updated quote
                let updated_quote = QuoteRow::find_by_id(conn, tenant_id, quote_id)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                updated_quote.try_into()
            }
            .scope_boxed()
        })
        .await
    }
}
