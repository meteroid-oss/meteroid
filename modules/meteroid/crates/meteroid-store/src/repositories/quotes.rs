use crate::StoreResult;
use crate::domain::{
    OrderByRequest, PaginatedVec, PaginationRequest, Quote, QuoteNew, QuoteWithCustomer,
    enums::QuoteStatusEnum,
    outbox_event::OutboxEvent,
    pgmq::{PgmqQueue, SendEmailRequest},
    quotes::{
        DetailedQuote, QuoteActivity, QuoteActivityNew, QuoteAddOn, QuoteAddOnNew, QuoteCoupon,
        QuoteCouponNew, QuotePriceComponent, QuotePriceComponentNew, QuoteSignature,
        QuoteSignatureNew,
    },
};
use crate::errors::StoreError;
use crate::jwt_claims::{ResourceAccess, generate_portal_token};
use crate::repositories::pgmq::PgmqInterface;
use crate::store::Store;
use common_domain::ids::{
    BaseId, CustomerId, QuoteId, QuotePriceComponentId, StoredDocumentId, TenantId,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_models::invoicing_entities::InvoicingEntityRow;
use diesel_models::quote_add_ons::{QuoteAddOnRow, QuoteAddOnRowNew};
use diesel_models::quote_coupons::{QuoteCouponRow, QuoteCouponRowNew};
use diesel_models::quotes::{
    QuoteActivityRow, QuoteActivityRowNew, QuoteComponentRow, QuoteComponentRowNew, QuoteRow,
    QuoteRowNew, QuoteRowUpdate, QuoteSignatureRow, QuoteSignatureRowNew,
};
use error_stack::Report;

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
        order_by: OrderByRequest,
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

    async fn accept_quote(&self, quote_id: QuoteId, tenant_id: TenantId) -> StoreResult<Quote>;

    async fn decline_quote(
        &self,
        quote_id: QuoteId,
        tenant_id: TenantId,
        reason: Option<String>,
    ) -> StoreResult<Quote>;

    async fn publish_quote(&self, quote_id: QuoteId, tenant_id: TenantId) -> StoreResult<Quote>;

    async fn insert_quote_signature(
        &self,
        signature: QuoteSignatureNew,
    ) -> StoreResult<QuoteSignature>;

    async fn list_quote_signatures(&self, quote_id: QuoteId) -> StoreResult<Vec<QuoteSignature>>;

    async fn insert_quote_activity(&self, activity: QuoteActivityNew)
    -> StoreResult<QuoteActivity>;

    async fn list_quote_activities(
        &self,
        quote_id: QuoteId,
        limit: Option<i64>,
    ) -> StoreResult<Vec<QuoteActivity>>;

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

    async fn insert_quote_add_ons(
        &self,
        add_ons: Vec<QuoteAddOnNew>,
    ) -> StoreResult<Vec<QuoteAddOn>>;

    async fn list_quote_add_ons(&self, quote_id: QuoteId) -> StoreResult<Vec<QuoteAddOn>>;

    async fn insert_quote_coupons(
        &self,
        coupons: Vec<QuoteCouponNew>,
    ) -> StoreResult<Vec<QuoteCoupon>>;

    async fn list_quote_coupons(&self, quote_id: QuoteId) -> StoreResult<Vec<QuoteCoupon>>;

    /// Creates a quote with all its related data (components, add-ons, coupons) in a single transaction.
    async fn insert_quote_with_details(
        &self,
        quote: QuoteNew,
        components: Vec<QuotePriceComponentNew>,
        add_ons: Vec<QuoteAddOnNew>,
        coupons: Vec<QuoteCouponNew>,
    ) -> StoreResult<Quote>;

    /// Cancels a quote, preventing future signature.
    /// Only quotes in Draft or Pending status can be cancelled.
    async fn cancel_quote(
        &self,
        quote_id: QuoteId,
        tenant_id: TenantId,
        reason: Option<String>,
    ) -> StoreResult<Quote>;

    /// Sends a quote to its recipients via email.
    /// This publishes the quote (sets status to Pending if in Draft) and queues the email.
    async fn send_quote(
        &self,
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

        let components = QuoteComponentRow::list_by_quote_id(&mut conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(|x| x.into_iter().map(TryInto::try_into).collect())?;

        let signatures = QuoteSignatureRow::list_by_quote_id(&mut conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|l| l.into_iter().map(std::convert::Into::into).collect())?;

        let activities = QuoteActivityRow::list_by_quote_id(&mut conn, quote_id, None)
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

        let add_ons = QuoteAddOnRow::list_by_quote_id(&mut conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(|x| x.into_iter().map(TryInto::try_into).collect())?;

        let coupons = QuoteCouponRow::list_by_quote_id(&mut conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|l| l.into_iter().map(std::convert::Into::into).collect())?;

        Ok(DetailedQuote {
            quote: quote_with_customer.quote,
            customer: quote_with_customer.customer,
            invoicing_entity,
            components,
            add_ons,
            coupons,
            signatures,
            activities,
        })
    }

    async fn list_quotes(
        &self,
        tenant_id: TenantId,
        customer_id: Option<CustomerId>,
        status: Option<QuoteStatusEnum>,
        search: Option<String>,
        order_by: OrderByRequest,
        pagination: PaginationRequest,
    ) -> StoreResult<PaginatedVec<QuoteWithCustomer>> {
        let mut conn = self.get_conn().await?;

        let rows = QuoteRow::list(
            &mut conn,
            tenant_id,
            customer_id,
            status.map(Into::into),
            search,
            order_by.into(),
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

    async fn accept_quote(&self, quote_id: QuoteId, tenant_id: TenantId) -> StoreResult<Quote> {
        self.transaction(|conn| {
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

                // Log activity
                let activity = QuoteActivityNew {
                    quote_id,
                    activity_type: "status_changed".to_string(),
                    description: "Quote accepted after all recipients signed".to_string(),
                    actor_type: "system".to_string(),
                    actor_id: None,
                    actor_name: None,
                    ip_address: None,
                    user_agent: None,
                };

                let activity_row: QuoteActivityRowNew = activity.into();
                activity_row
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // Emit QuoteAccepted event if create_subscription_on_acceptance is true
                let should_create_subscription = updated_row.create_subscription_on_acceptance;
                let quote: Quote = updated_row.try_into()?;

                if should_create_subscription {
                    self.internal
                        .insert_outbox_events_tx(
                            conn,
                            vec![OutboxEvent::quote_accepted(quote.clone().into())],
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
        quote_id: QuoteId,
        tenant_id: TenantId,
        reason: Option<String>, // TODO save it in quote ?
    ) -> StoreResult<Quote> {
        self.transaction(|conn| {
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

                // Log activity
                let description = reason.map_or("Quote declined".to_string(), |r| {
                    format!("Quote declined: {r}")
                });
                let activity = QuoteActivityNew {
                    quote_id,
                    activity_type: "declined".to_string(),
                    description,
                    actor_type: "customer".to_string(),
                    actor_id: None,
                    actor_name: None,
                    ip_address: None,
                    user_agent: None,
                };

                let activity_row: QuoteActivityRowNew = activity.into();
                activity_row
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                updated_row.try_into()
            }
            .scope_boxed()
        })
        .await
    }

    async fn publish_quote(&self, quote_id: QuoteId, tenant_id: TenantId) -> StoreResult<Quote> {
        self.transaction(|conn| {
            async move {
                let now = chrono::Utc::now().naive_utc();

                // Update quote status to Pending
                let update = QuoteRowUpdate {
                    status: Some(diesel_models::enums::QuoteStatusEnum::Pending),
                    updated_at: Some(now),
                    ..Default::default()
                };

                let updated_row = QuoteRow::update_by_id(conn, tenant_id, quote_id, update)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                // Log activity
                let activity = QuoteActivityNew {
                    quote_id,
                    activity_type: "published".to_string(),
                    description: "Quote published and made available to recipients".to_string(),
                    actor_type: "user".to_string(),
                    actor_id: None,
                    actor_name: None,
                    ip_address: None,
                    user_agent: None,
                };

                let activity_row: QuoteActivityRowNew = activity.into();
                activity_row
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                updated_row.try_into()
            }
            .scope_boxed()
        })
        .await
    }

    async fn insert_quote_signature(
        &self,
        signature: QuoteSignatureNew,
    ) -> StoreResult<QuoteSignature> {
        self.transaction(|conn| {
            async move {
                let activity = QuoteActivityNew {
                    quote_id: signature.quote_id,
                    activity_type: "signature_added".to_string(),
                    description: format!("Quote signed by {}", signature.signed_by_name.clone()),
                    actor_type: "recipient".to_string(),
                    actor_id: Some(signature.signed_by_email.clone()),
                    actor_name: Some(signature.signed_by_name.clone()),
                    ip_address: signature.ip_address.clone(),
                    user_agent: signature.user_agent.clone(),
                };
                let activity_row: QuoteActivityRowNew = activity.into();
                activity_row
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

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

    async fn insert_quote_activity(
        &self,
        activity: QuoteActivityNew,
    ) -> StoreResult<QuoteActivity> {
        let mut conn = self.get_conn().await?;

        let activity_row: QuoteActivityRowNew = activity.into();
        activity_row
            .insert(&mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(std::convert::Into::into)
    }

    async fn list_quote_activities(
        &self,
        quote_id: QuoteId,
        limit: Option<i64>,
    ) -> StoreResult<Vec<QuoteActivity>> {
        let mut conn = self.get_conn().await?;

        QuoteActivityRow::list_by_quote_id(&mut conn, quote_id, limit)
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

    async fn insert_quote_add_ons(
        &self,
        add_ons: Vec<QuoteAddOnNew>,
    ) -> StoreResult<Vec<QuoteAddOn>> {
        let mut conn = self.get_conn().await?;

        let rows_new: Vec<QuoteAddOnRowNew> = add_ons
            .into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;

        let rows = QuoteAddOnRowNew::insert_batch(&rows_new, &mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        rows.into_iter()
            .map(std::convert::TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()
    }

    async fn list_quote_add_ons(&self, quote_id: QuoteId) -> StoreResult<Vec<QuoteAddOn>> {
        let mut conn = self.get_conn().await?;

        QuoteAddOnRow::list_by_quote_id(&mut conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .and_then(|rows| {
                rows.into_iter()
                    .map(std::convert::TryInto::try_into)
                    .collect::<Result<Vec<_>, _>>()
            })
    }

    async fn insert_quote_coupons(
        &self,
        coupons: Vec<QuoteCouponNew>,
    ) -> StoreResult<Vec<QuoteCoupon>> {
        let mut conn = self.get_conn().await?;

        let rows_new: Vec<QuoteCouponRowNew> =
            coupons.into_iter().map(std::convert::Into::into).collect();

        let rows = QuoteCouponRowNew::insert_batch(&rows_new, &mut conn)
            .await
            .map_err(Into::<Report<StoreError>>::into)?;

        Ok(rows.into_iter().map(std::convert::Into::into).collect())
    }

    async fn list_quote_coupons(&self, quote_id: QuoteId) -> StoreResult<Vec<QuoteCoupon>> {
        let mut conn = self.get_conn().await?;

        QuoteCouponRow::list_by_quote_id(&mut conn, quote_id)
            .await
            .map_err(Into::<Report<StoreError>>::into)
            .map(|rows| rows.into_iter().map(std::convert::Into::into).collect())
    }

    async fn insert_quote_with_details(
        &self,
        quote: QuoteNew,
        components: Vec<QuotePriceComponentNew>,
        add_ons: Vec<QuoteAddOnNew>,
        coupons: Vec<QuoteCouponNew>,
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
                            fee: serde_json::to_value(&c.fee).unwrap_or_default(),
                            is_override: c.is_override,
                        })
                        .collect();

                    QuoteComponentRowNew::insert_batch(&component_rows, conn)
                        .await
                        .map_err(Into::<Report<StoreError>>::into)?;
                }

                // Insert add-ons if any
                if !add_ons.is_empty() {
                    let add_on_rows: Vec<QuoteAddOnRowNew> = add_ons
                        .into_iter()
                        .map(std::convert::TryInto::try_into)
                        .collect::<Result<Vec<_>, _>>()?;

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

                created_quote.try_into()
            }
            .scope_boxed()
        })
        .await
    }

    async fn cancel_quote(
        &self,
        quote_id: QuoteId,
        tenant_id: TenantId,
        reason: Option<String>,
    ) -> StoreResult<Quote> {
        self.transaction(|conn| {
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

                // Log activity
                let description = reason.map_or("Quote cancelled".to_string(), |r| {
                    format!("Quote cancelled: {r}")
                });
                let activity = QuoteActivityNew {
                    quote_id,
                    activity_type: "cancelled".to_string(),
                    description,
                    actor_type: "user".to_string(),
                    actor_id: None,
                    actor_name: None,
                    ip_address: None,
                    user_agent: None,
                };

                let activity_row: QuoteActivityRowNew = activity.into();
                activity_row
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

                updated_row.try_into()
            }
            .scope_boxed()
        })
        .await
    }

    async fn send_quote(
        // TODO rename publish_and_send ?
        &self,
        quote_id: QuoteId,
        tenant_id: TenantId,
        custom_message: Option<String>,
    ) -> StoreResult<Quote> {
        self.transaction(|conn| {
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

                // Log activity
                let activity = QuoteActivityNew {
                    quote_id,
                    activity_type: "sent".to_string(),
                    description: "Quote sent to recipients via email".to_string(),
                    actor_type: "user".to_string(),
                    actor_id: None,
                    actor_name: None,
                    ip_address: None,
                    user_agent: None,
                };
                let activity_row: QuoteActivityRowNew = activity.into();
                activity_row
                    .insert(conn)
                    .await
                    .map_err(Into::<Report<StoreError>>::into)?;

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
