use super::{QuoteServiceComponents, mapping};
use crate::entitlements::mapping::entitlement_spec_from_proto;
use crate::quotes::error::QuoteApiError;
use common_domain::ids::{
    AddOnId, BaseId, CouponId, CustomerId, PlanVersionId, PriceId, ProductId, QuoteId,
};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::conversions::FromProtoOpt;
use meteroid_app::features::{minimum_commitments_open, minimum_commitments_refused};
use meteroid_grpc::meteroid::api::quotes::v1::{
    CancelQuoteRequest, CancelQuoteResponse, ConvertQuoteToSubscriptionRequest,
    ConvertQuoteToSubscriptionResponse, CreateQuote, CreateQuoteRequest, CreateQuoteResponse,
    DuplicateQuoteRequest, DuplicateQuoteResponse, ExpireQuoteRequest, ExpireQuoteResponse,
    GenerateQuotePortalTokenRequest, GenerateQuotePortalTokenResponse, GetQuoteRequest,
    GetQuoteResponse, ListQuotesRequest, ListQuotesResponse, PreviewQuoteRequest,
    PreviewQuoteResponse, PublishQuoteRequest, PublishQuoteResponse, SendQuoteRequest,
    SendQuoteResponse, UpdateQuoteRequest, UpdateQuoteResponse,
    quotes_service_server::QuotesService,
};
use meteroid_grpc_mapping::utils::PaginationExt;
use meteroid_store::domain::add_ons::AddOn;
use meteroid_store::domain::customer_emails::EmailKind;
use meteroid_store::domain::quotes::{QuoteAddOnNew, QuoteCouponNew};
use meteroid_store::domain::{CreateSubscriptionAddOns, CreateSubscriptionComponents};
use meteroid_store::domain::{PriceComponent, quotes::QuotePriceComponentNew};
use meteroid_store::repositories::CreditUnitsInterface;
use meteroid_store::repositories::QuotesInterface;
use meteroid_store::repositories::customer_emails::CustomerEmailsInterface;
use nanoid::nanoid;
use tonic::{Request, Response, Status};

use crate::subscriptions::mapping::add_ons::create_subscription_add_ons_from_grpc;
use crate::subscriptions::mapping::price_components::create_subscription_components_from_grpc;
use common_utils::rng::UPPER_ALPHANUMERIC;
use meteroid_store::repositories::add_ons::AddOnInterfaceAuto;
use meteroid_store::repositories::price_components::PriceComponentInterfaceAuto;
use meteroid_store::repositories::prices::PriceInterface;
use meteroid_store::repositories::products::ProductInterface;

#[tonic::async_trait]
impl QuotesService for QuoteServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn create_quote(
        &self,
        request: Request<CreateQuoteRequest>,
    ) -> Result<Response<CreateQuoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let quote = inner
            .quote
            .ok_or_else(|| Status::invalid_argument("quote is required"))?;

        let quote_id = QuoteId::new();
        let prepared = self
            .prepare_quote(
                tenant_id,
                quote_id,
                quote,
                format!("Q-{}", nanoid!(8, &UPPER_ALPHANUMERIC)),
            )
            .await?;
        if prepared.minimum.is_some()
            && !minimum_commitments_open(self.feature_resolver.as_ref(), organization_id).await
        {
            return Err(minimum_commitments_refused());
        }

        let created_quote = self
            .store
            .insert_quote_with_details(
                prepared.quote_new,
                prepared.components,
                prepared.add_ons,
                prepared.coupons,
                prepared.pending_addon_materializations,
                actor,
            )
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        if let Some((amount_subunit, scope)) = prepared.minimum {
            self.store
                .set_quote_minimum(tenant_id, created_quote.id, amount_subunit, scope)
                .await
                .map_err(Into::<QuoteApiError>::into)?;
        }

        let detailed_quote = self
            .store
            .get_detailed_quote_by_id(tenant_id, created_quote.id)
            .await
            .map_err(Into::<QuoteApiError>::into)
            .map(|q| mapping::quotes::detailed_quote_domain_to_proto(&q))?;

        Ok(Response::new(CreateQuoteResponse {
            quote: Some(detailed_quote),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_quote(
        &self,
        request: Request<GetQuoteRequest>,
    ) -> Result<Response<GetQuoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let quote_id = QuoteId::from_proto(&inner.id)?;

        let detailed_quote_domain = self
            .store
            .get_detailed_quote_by_id(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        let email_settings = self
            .store
            .get_email_settings(tenant_id, detailed_quote_domain.invoicing_entity.id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        let mut detailed_quote =
            mapping::quotes::detailed_quote_domain_to_proto(&detailed_quote_domain);
        detailed_quote.quote_email_enabled = Some(email_settings.is_enabled(EmailKind::QuoteReady));

        Ok(Response::new(GetQuoteResponse {
            quote: Some(detailed_quote),
        }))
    }

    #[tracing::instrument(skip_all, fields(customer_id = request.get_ref().customer_id.as_deref()))]
    async fn list_quotes(
        &self,
        request: Request<ListQuotesRequest>,
    ) -> Result<Response<ListQuotesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let customer_id = CustomerId::from_proto_opt(inner.customer_id)?;
        let pagination_req = inner.pagination.into_domain();

        let status = mapping::quotes::status_server_to_domain(inner.status);

        let quotes = self
            .store
            .list_quotes(
                tenant_id,
                customer_id,
                status,
                inner.search,
                meteroid_grpc_mapping::filters::map_filters(inner.filters)?,
                inner.order_by,
                pagination_req,
            )
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        let proto_quotes = quotes
            .items
            .into_iter()
            .map(|quote_with_customer| {
                mapping::quotes::quote_to_proto(
                    &quote_with_customer.quote,
                    Some(quote_with_customer.customer.name),
                    false,
                )
            })
            .collect();

        Ok(Response::new(ListQuotesResponse {
            quotes: proto_quotes,
            pagination_meta: inner
                .pagination
                .into_response(quotes.total_pages, quotes.total_results),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn send_quote(
        &self,
        request: Request<SendQuoteRequest>,
    ) -> Result<Response<SendQuoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let quote_id = QuoteId::from_proto(&inner.id)?;

        // Send the quote (publishes if draft, queues email)
        let _updated_quote = self
            .store
            .send_quote(actor, quote_id, tenant_id, inner.message)
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        Ok(Response::new(SendQuoteResponse {
            success: true,
            message: Some("Quote sent successfully".to_string()),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn preview_quote_html(
        &self,
        _request: Request<PreviewQuoteRequest>,
    ) -> Result<Response<PreviewQuoteResponse>, Status> {
        unimplemented!()
    }

    #[tracing::instrument(skip_all)]
    async fn expire_quote(
        &self,
        _request: Request<ExpireQuoteRequest>,
    ) -> Result<Response<ExpireQuoteResponse>, Status> {
        unimplemented!()
    }

    #[tracing::instrument(skip_all)]
    async fn cancel_quote(
        &self,
        request: Request<CancelQuoteRequest>,
    ) -> Result<Response<CancelQuoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let quote_id = QuoteId::from_proto(&inner.id)?;

        // Cancel the quote
        let updated_quote = self
            .store
            .cancel_quote(actor, quote_id, tenant_id, inner.reason)
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        // Get the detailed quote for the response
        let detailed_quote = self
            .store
            .get_detailed_quote_by_id(tenant_id, updated_quote.id)
            .await
            .map_err(Into::<QuoteApiError>::into)
            .map(|q| mapping::quotes::detailed_quote_domain_to_proto(&q))?;

        Ok(Response::new(CancelQuoteResponse {
            quote: Some(detailed_quote),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn duplicate_quote(
        &self,
        _request: Request<DuplicateQuoteRequest>,
    ) -> Result<Response<DuplicateQuoteResponse>, Status> {
        unimplemented!()
    }

    #[tracing::instrument(skip_all)]
    async fn update_quote(
        &self,
        request: Request<UpdateQuoteRequest>,
    ) -> Result<Response<UpdateQuoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let quote_id = QuoteId::from_proto(&inner.id)?;
        let quote = inner
            .quote
            .ok_or_else(|| Status::invalid_argument("quote is required"))?;

        let existing = self
            .store
            .get_quote_by_id(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        if !matches!(
            existing.status,
            meteroid_store::domain::enums::QuoteStatusEnum::Draft
                | meteroid_store::domain::enums::QuoteStatusEnum::Pending
        ) {
            return Err(Status::failed_precondition(
                "Only draft or pending quotes can be edited",
            ));
        }

        let prepared = self
            .prepare_quote(tenant_id, quote_id, quote, existing.quote_number)
            .await?;
        if let Some((amount_subunit, scope)) = &prepared.minimum
            && !minimum_commitments_open(self.feature_resolver.as_ref(), organization_id).await
        {
            let kept = self
                .store
                .get_quote_minimum(tenant_id, quote_id)
                .await
                .map_err(Into::<QuoteApiError>::into)?
                .is_some_and(|stored| {
                    stored.amount_subunit == *amount_subunit && stored.create_scope().same_as(scope)
                });
            if !kept {
                return Err(minimum_commitments_refused());
            }
        }

        self.store
            .replace_quote(
                prepared.quote_new,
                prepared.components,
                prepared.add_ons,
                prepared.coupons,
                prepared.pending_addon_materializations,
                prepared.minimum,
                actor,
            )
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        let detailed_quote = self
            .store
            .get_detailed_quote_by_id(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)
            .map(|q| mapping::quotes::detailed_quote_domain_to_proto(&q))?;

        Ok(Response::new(UpdateQuoteResponse {
            quote: Some(detailed_quote),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn generate_quote_portal_token(
        &self,
        request: Request<GenerateQuotePortalTokenRequest>,
    ) -> Result<Response<GenerateQuotePortalTokenResponse>, Status> {
        let tenant_id = request.tenant()?;
        let delegated = request.delegated();
        let inner = request.into_inner();

        let quote_id = QuoteId::from_proto(&inner.quote_id)?;

        // Verify the quote exists and the recipient is valid
        let quote = self
            .store
            .get_quote_by_id(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        // Verify recipient is in the quote recipients list
        let is_valid_recipient = quote
            .recipients
            .iter()
            .any(|r| r.email == inner.recipient_email);
        if !is_valid_recipient {
            return Err(Status::invalid_argument(
                "Recipient not found in quote recipients",
            ));
        }

        // Generate portal token
        let token = meteroid_store::jwt_claims::generate_portal_token(
            &self.jwt_secret,
            tenant_id,
            meteroid_store::jwt_claims::ResourceAccess::Quote {
                quote_id,
                recipient_email: inner.recipient_email.clone(),
            },
            None,
            None,
            delegated,
        )
        .map_err(Into::<QuoteApiError>::into)?;

        Ok(Response::new(GenerateQuotePortalTokenResponse { token }))
    }

    #[tracing::instrument(skip_all)]
    async fn publish_quote(
        &self,
        request: Request<PublishQuoteRequest>,
    ) -> Result<Response<PublishQuoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let quote_id = QuoteId::from_proto(&inner.id)?;

        let updated_quote = self
            .store
            .publish_quote(actor, quote_id, tenant_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        let detailed_quote = self
            .store
            .get_detailed_quote_by_id(tenant_id, updated_quote.id)
            .await
            .map_err(Into::<QuoteApiError>::into)
            .map(|q| mapping::quotes::detailed_quote_domain_to_proto(&q))?;

        Ok(Response::new(PublishQuoteResponse {
            quote: Some(detailed_quote),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn convert_quote_to_subscription(
        &self,
        request: Request<ConvertQuoteToSubscriptionRequest>,
    ) -> Result<Response<ConvertQuoteToSubscriptionResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor_typed = request.actor_typed()?;
        let inner = request.into_inner();

        let quote_id = QuoteId::from_proto(&inner.quote_id)?;

        // Convert the quote to a subscription
        let result = self
            .services
            .convert_quote_to_subscription(actor_typed, tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        // Get the updated quote
        let updated_quote = self
            .store
            .get_quote_by_id(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        // Map the subscription to proto
        let subscription = crate::subscriptions::mapping::subscriptions::created_domain_to_proto(
            result.subscription,
        )?;

        Ok(Response::new(ConvertQuoteToSubscriptionResponse {
            quote: Some(mapping::quotes::quote_to_proto(&updated_quote, None, false)),
            subscription: Some(subscription),
        }))
    }
}

/// Accepts RFC 3339 timestamps as well as the plain `yyyy-MM-dd` dates sent by the date pickers.
fn parse_quote_datetime(s: &str) -> Option<chrono::NaiveDateTime> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.naive_utc())
        .ok()
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        })
}

struct PreparedQuote {
    quote_new: meteroid_store::domain::quotes::QuoteNew,
    components: Vec<QuotePriceComponentNew>,
    add_ons: Vec<QuoteAddOnNew>,
    coupons: Vec<QuoteCouponNew>,
    pending_addon_materializations: Vec<meteroid_store::services::PendingMaterialization>,
    minimum: Option<(
        i64,
        meteroid_store::domain::plan_minimums::CreateMinimumScope,
    )>,
}

impl QuoteServiceComponents {
    async fn prepare_quote(
        &self,
        tenant_id: common_domain::ids::TenantId,
        quote_id: QuoteId,
        quote: CreateQuote,
        default_quote_number: String,
    ) -> Result<PreparedQuote, Status> {
        let customer_id = CustomerId::from_proto(&quote.customer_id)?;
        let plan_version_id = PlanVersionId::from_proto(&quote.plan_version_id)?;

        // Parsed with the create-time parser so inline new-product extras (by name) survive to
        // conversion, where the created subscription's `minimum_override` resolves them.
        let minimum = quote
            .minimum_commitment_override
            .clone()
            .map(crate::plans::mapping::plans::create_minimum_from_proto)
            .transpose()
            .map_err(Status::invalid_argument)?;

        // Map activation condition from proto to domain
        let activation_condition =
            mapping::quotes::activation_condition_to_domain(quote.activation_condition());

        let recipients = quote
            .recipients
            .into_iter()
            .map(mapping::quotes::recipient_details_to_domain)
            .collect();

        // Parse optional start_date
        let billing_start_date = quote
            .start_date
            .and_then(|s| chrono::NaiveDate::from_proto_opt(Some(s)).ok())
            .flatten();

        let custom_properties = quote
            .custom_properties
            .as_deref()
            .map(serde_json::from_str::<serde_json::Value>)
            .transpose()
            .map_err(|e| {
                QuoteApiError::InvalidArgument(format!("Invalid custom_properties JSON: {e}"))
            })?
            .unwrap_or(serde_json::Value::Null);

        let requested_net_terms = quote.net_terms;

        let mut quote_new = meteroid_store::domain::quotes::QuoteNew {
            id: quote_id,
            status: meteroid_store::domain::enums::QuoteStatusEnum::Draft,
            tenant_id,
            customer_id,
            plan_version_id,
            currency: quote.currency,
            quote_number: quote.quote_number.unwrap_or(default_quote_number),
            // Subscription-like fields
            trial_duration_days: quote.trial_duration.map(|d| d as i32),
            billing_start_date,
            billing_end_date: quote
                .end_date
                .and_then(|s| chrono::NaiveDate::from_proto_opt(Some(s)).ok())
                .flatten(),
            billing_day_anchor: quote.billing_day_anchor.map(|d| d as i32),
            activation_condition,
            // Quote-specific fields
            valid_until: quote.valid_until.as_deref().and_then(parse_quote_datetime),
            expires_at: quote.expires_at.as_deref().and_then(parse_quote_datetime),
            internal_notes: quote.internal_notes,
            cover_image: quote.cover_image.and_then(|s| s.parse().ok()),
            overview: quote.overview,
            terms_and_services: quote.terms_and_services,
            // Set once the quote's components are resolved, when the request names none.
            net_terms: requested_net_terms.unwrap_or_default(),
            attachments: quote
                .attachments
                .into_iter()
                .filter_map(|s| s.parse().ok())
                .map(Some)
                .collect(),
            pdf_document_id: None,
            sharing_key: Some(uuid::Uuid::new_v4().to_string()),
            recipients,
            // Payment configuration fields
            auto_advance_invoices: quote.auto_advance_invoices.unwrap_or(true),
            charge_automatically: quote.charge_automatically.unwrap_or(true),
            invoice_memo: quote.invoice_memo,
            invoice_threshold: quote
                .invoice_threshold
                .and_then(|s| s.parse::<rust_decimal::Decimal>().ok()),
            create_subscription_on_acceptance: quote
                .create_subscription_on_acceptance
                .unwrap_or(false),
            payment_methods_config: mapping::quotes::payment_methods_config_to_domain(
                quote.payment_methods_config,
            ),
            custom_properties,
            entitlements: quote
                .entitlements
                .into_iter()
                .map(entitlement_spec_from_proto)
                .collect::<Result<Vec<_>, _>>()?,
        };

        // Parse informative usage example quantities (display-only).
        // Components are matched by plan price_component_id when present, else by name.
        let mut examples_by_component_id: std::collections::HashMap<
            common_domain::ids::PriceComponentId,
            rust_decimal::Decimal,
        > = std::collections::HashMap::new();
        let mut examples_by_name: std::collections::HashMap<String, rust_decimal::Decimal> =
            std::collections::HashMap::new();
        for ex in &quote.usage_examples {
            let qty = ex
                .example_quantity
                .parse::<rust_decimal::Decimal>()
                .map_err(|_| {
                    Status::invalid_argument(format!(
                        "Invalid example_quantity '{}'",
                        ex.example_quantity
                    ))
                })?;
            if let Some(pc_id) = &ex.price_component_id {
                examples_by_component_id.insert(
                    common_domain::ids::PriceComponentId::from_proto(pc_id)?,
                    qty,
                );
            } else if let Some(name) = &ex.component_name {
                examples_by_name.insert(name.clone(), qty);
            }
        }

        // Process quote components (fetch plan price components + products + prices first)
        let mut quote_components = if let Some(components) = quote.components {
            let price_components = self
                .store
                .list_price_components(plan_version_id, tenant_id)
                .await
                .map_err(Into::<QuoteApiError>::into)?;

            let create_components = create_subscription_components_from_grpc(components)?;

            // Load products referenced by price components and extra components
            let pc_product_ids: Vec<ProductId> = price_components
                .iter()
                .filter_map(|c| c.product_id)
                .chain(create_components.extra_components.iter().filter_map(|ex| {
                    if let meteroid_store::domain::price_components::ProductRef::Existing(pid) =
                        &ex.product_ref
                    {
                        Some(*pid)
                    } else {
                        None
                    }
                }))
                .collect();

            let products_map: std::collections::HashMap<ProductId, _> = if pc_product_ids.is_empty()
            {
                std::collections::HashMap::new()
            } else {
                self.store
                    .find_products_by_ids(&pc_product_ids, tenant_id)
                    .await
                    .map_err(Into::<QuoteApiError>::into)?
                    .into_iter()
                    .map(|p| (p.id, p))
                    .collect()
            };

            // Batch-load all existing prices from overrides and extras
            let existing_price_ids: Vec<PriceId> = create_components
                .overridden_components
                .iter()
                .filter_map(|ov| ov.price_entry.existing_price_id())
                .chain(
                    create_components
                        .extra_components
                        .iter()
                        .filter_map(|ex| ex.price_entry.existing_price_id()),
                )
                .collect();

            let prices_map: std::collections::HashMap<PriceId, _> = if existing_price_ids.is_empty()
            {
                std::collections::HashMap::new()
            } else {
                self.store
                    .list_prices_by_ids(&existing_price_ids, tenant_id)
                    .await
                    .map_err(Into::<QuoteApiError>::into)?
                    .into_iter()
                    .map(|p| (p.id, p))
                    .collect()
            };

            let units = self
                .store
                .get_credit_units_by_ids(
                    tenant_id,
                    price_components
                        .iter()
                        .flat_map(|pc| pc.credit_unit_ids())
                        .chain(
                            prices_map
                                .values()
                                .flat_map(meteroid_store::domain::Price::credit_unit_ids),
                        )
                        .chain(
                            create_components
                                .overridden_components
                                .iter()
                                .flat_map(|ov| ov.price_entry.new_credit_unit_ids()),
                        )
                        .chain(
                            create_components
                                .extra_components
                                .iter()
                                .flat_map(|ex| ex.price_entry.new_credit_unit_ids()),
                        )
                        .collect(),
                )
                .await
                .map_err(Into::<QuoteApiError>::into)?;

            process_quote_components(
                &create_components,
                &price_components,
                &products_map,
                &prices_map,
                &units,
                quote_id,
            )?
        } else {
            vec![]
        };

        // Attach informative example usage quantities to usage-based components only.
        if !examples_by_component_id.is_empty() || !examples_by_name.is_empty() {
            for component in quote_components.iter_mut() {
                if matches!(
                    component.fee,
                    meteroid_store::domain::SubscriptionFee::Usage { .. }
                ) {
                    component.example_usage_quantity = component
                        .price_component_id
                        .and_then(|id| examples_by_component_id.get(&id).copied())
                        .or_else(|| examples_by_name.get(&component.name).copied());
                }
            }
        }

        // Load plan info for product_family_id (needed for add-on materialization)
        use meteroid_store::repositories::plans::PlansInterfaceAuto;
        let plan_with_version = self
            .store
            .get_plan_by_version_id(plan_version_id, tenant_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        let plan_version = plan_with_version
            .version
            .as_ref()
            .ok_or_else(|| QuoteApiError::InvalidArgument("Plan version not found".to_string()))?;

        // Process quote add-ons (fetch add-on details first)
        let (quote_add_ons, pending_addon_materializations) = if let Some(add_ons_proto) =
            quote.add_ons
        {
            let create_add_ons = create_subscription_add_ons_from_grpc(add_ons_proto)?;

            if !create_add_ons.add_ons.is_empty() {
                let add_on_ids: Vec<AddOnId> =
                    create_add_ons.add_ons.iter().map(|a| a.add_on_id).collect();

                let add_ons = self
                    .store
                    .list_add_ons_by_ids(tenant_id, add_on_ids)
                    .await
                    .map_err(Into::<QuoteApiError>::into)?;

                // Collect product_ids and price_ids from add-ons for fee resolution
                let product_ids: Vec<ProductId> = add_ons.iter().map(|a| a.product_id).collect();
                let price_ids: Vec<PriceId> = add_ons.iter().map(|a| a.price_id).collect();

                let products = self
                    .store
                    .find_products_by_ids(&product_ids, tenant_id)
                    .await
                    .map_err(Into::<QuoteApiError>::into)?;
                let products_map: std::collections::HashMap<ProductId, _> =
                    products.into_iter().map(|p| (p.id, p)).collect();

                let prices = self
                    .store
                    .list_prices_by_ids(&price_ids, tenant_id)
                    .await
                    .map_err(Into::<QuoteApiError>::into)?;
                let prices_map: std::collections::HashMap<PriceId, _> =
                    prices.into_iter().map(|p| (p.id, p)).collect();

                let units = self
                    .store
                    .get_credit_units_by_ids(
                        tenant_id,
                        prices_map
                            .values()
                            .flat_map(meteroid_store::domain::Price::credit_unit_ids)
                            .chain(
                                create_add_ons
                                    .add_ons
                                    .iter()
                                    .flat_map(|a| a.customization.new_credit_unit_ids()),
                            )
                            .collect(),
                    )
                    .await
                    .map_err(Into::<QuoteApiError>::into)?;

                process_quote_add_ons(
                    &create_add_ons,
                    &add_ons,
                    &products_map,
                    &prices_map,
                    &units,
                    quote_id,
                    plan_with_version.plan.product_family_id,
                    &plan_version.currency,
                )?
            } else {
                (vec![], vec![])
            }
        } else {
            (vec![], vec![])
        };

        // Process quote coupons
        let quote_coupons: Vec<QuoteCouponNew> = if let Some(coupons_proto) = quote.coupons {
            coupons_proto
                .coupons
                .iter()
                .filter_map(|c| {
                    CouponId::from_proto_opt(Some(c.coupon_id.clone()))
                        .ok()
                        .flatten()
                })
                .map(|coupon_id| QuoteCouponNew {
                    quote_id,
                    coupon_id,
                })
                .collect()
        } else {
            vec![]
        };

        if requested_net_terms.is_none() {
            // Without customized components the quote bills the plan's own.
            let sells_credit = if quote_components.is_empty() {
                self.store
                    .list_price_components(plan_version_id, tenant_id)
                    .await
                    .map_err(Into::<QuoteApiError>::into)?
                    .iter()
                    .any(PriceComponent::sells_credit)
            } else {
                quote_components.iter().any(|c| c.fee.sells_credit())
            } || quote_add_ons.iter().any(|a| a.fee.sells_credit());
            let net_terms = meteroid_store::domain::subscriptions::default_net_terms(
                sells_credit,
                u32::try_from(plan_version.net_terms).unwrap_or(0),
            );
            quote_new.net_terms = i32::try_from(net_terms).unwrap_or(plan_version.net_terms);
        }

        Ok(PreparedQuote {
            quote_new,
            components: quote_components,
            add_ons: quote_add_ons,
            coupons: quote_coupons,
            pending_addon_materializations,
            minimum,
        })
    }
}

fn process_quote_components(
    components: &CreateSubscriptionComponents,
    price_components: &[PriceComponent],
    products: &std::collections::HashMap<ProductId, meteroid_store::domain::Product>,
    prices: &std::collections::HashMap<PriceId, meteroid_store::domain::Price>,
    units: &meteroid_store::domain::credit_units::CreditUnitsById,
    quote_id: QuoteId,
) -> Result<Vec<QuotePriceComponentNew>, Status> {
    use meteroid_store::domain::price_components::{ComponentParameters, PriceEntry, ProductRef};
    use meteroid_store::domain::prices::resolve_fee_from_entry;

    let mut processed_components = Vec::new();

    // Process parameterized components
    for parameterized in &components.parameterized_components {
        if let Some(price_component) = price_components
            .iter()
            .find(|pc| pc.id == parameterized.component_id)
        {
            let params = ComponentParameters {
                initial_slot_count: parameterized.parameters.initial_slot_count,
                billing_period: parameterized.parameters.billing_period,
                committed_capacity: parameterized.parameters.committed_capacity,
            };
            let resolved = price_component
                .resolve_subscription_fee(products, units, Some(&params))
                .map_err(|e| Status::internal(format!("Failed to process component fee: {e}")))?;

            processed_components.push(QuotePriceComponentNew {
                name: price_component.name.clone(),
                quote_id,
                price_component_id: Some(price_component.id),
                product_id: price_component.product_id,
                period: resolved.period,
                fee: resolved.fee,
                is_override: false,
                price_id: resolved.price_id,
                example_usage_quantity: None,
            });
        }
    }

    // Process overridden components
    for overridden in &components.overridden_components {
        if let Some(price_component) = price_components
            .iter()
            .find(|pc| pc.id == overridden.component_id)
        {
            let product_id = price_component.product_id.ok_or_else(|| {
                Status::invalid_argument(format!(
                    "Cannot override component {} — it has no product_id",
                    price_component.id
                ))
            })?;

            let product = products.get(&product_id).ok_or_else(|| {
                Status::internal(format!(
                    "Product {} not found for override component {}",
                    product_id, price_component.id
                ))
            })?;

            let (fee, period) = resolve_fee_from_entry(
                &product.fee_structure,
                &overridden.price_entry,
                prices,
                units,
            )
            .map_err(|e| Status::internal(format!("Failed to resolve override fee: {e}")))?;

            processed_components.push(QuotePriceComponentNew {
                name: overridden.name.clone(),
                quote_id,
                price_component_id: Some(price_component.id),
                product_id: Some(product_id),
                period,
                fee,
                is_override: true,
                price_id: overridden.price_entry.existing_price_id(),
                example_usage_quantity: None,
            });
        }
    }

    // Process extra components
    for extra in &components.extra_components {
        let fee_structure = match &extra.product_ref {
            ProductRef::Existing(pid) => {
                let product = products.get(pid).ok_or_else(|| {
                    Status::internal(format!("Product {} not found for extra component", pid))
                })?;
                product.fee_structure.clone()
            }
            ProductRef::New { fee_structure, .. } => fee_structure.clone(),
        };

        if matches!(extra.product_ref, ProductRef::New { .. })
            && matches!(extra.price_entry, PriceEntry::Existing(_))
        {
            return Err(Status::invalid_argument(
                "Cannot use existing price with a new product",
            ));
        }

        let (fee, period) =
            resolve_fee_from_entry(&fee_structure, &extra.price_entry, prices, units).map_err(
                |e| Status::internal(format!("Failed to resolve extra component fee: {e}")),
            )?;

        processed_components.push(QuotePriceComponentNew {
            name: extra.name.clone(),
            quote_id,
            price_component_id: None,
            product_id: extra.product_ref.existing_product_id(),
            period,
            fee,
            is_override: false,
            price_id: extra.price_entry.existing_price_id(),
            example_usage_quantity: None,
        });
    }

    // Process components that are not removed or customized (default plan components)
    let configured_component_ids: std::collections::HashSet<common_domain::ids::PriceComponentId> =
        components
            .parameterized_components
            .iter()
            .map(|p| p.component_id)
            .chain(
                components
                    .overridden_components
                    .iter()
                    .map(|o| o.component_id),
            )
            .collect();

    for price_component in price_components {
        // Skip if component is removed, parameterized, or overridden
        if components.remove_components.contains(&price_component.id)
            || configured_component_ids.contains(&price_component.id)
        {
            continue;
        }

        let resolved = price_component
            .resolve_subscription_fee(products, units, None)
            .map_err(|e| {
                Status::internal(format!("Failed to process default component fee: {e}"))
            })?;

        processed_components.push(QuotePriceComponentNew {
            name: price_component.name.clone(),
            quote_id,
            price_component_id: Some(price_component.id),
            product_id: price_component.product_id,
            period: resolved.period,
            fee: resolved.fee,
            is_override: false,
            price_id: resolved.price_id,
            example_usage_quantity: None,
        });
    }

    Ok(processed_components)
}

fn process_quote_add_ons(
    create_add_ons: &CreateSubscriptionAddOns,
    add_ons: &[AddOn],
    products: &std::collections::HashMap<
        common_domain::ids::ProductId,
        meteroid_store::domain::Product,
    >,
    prices: &std::collections::HashMap<common_domain::ids::PriceId, meteroid_store::domain::Price>,
    units: &meteroid_store::domain::credit_units::CreditUnitsById,
    quote_id: QuoteId,
    product_family_id: common_domain::ids::ProductFamilyId,
    currency: &str,
) -> Result<
    (
        Vec<QuoteAddOnNew>,
        Vec<meteroid_store::services::PendingMaterialization>,
    ),
    Status,
> {
    use meteroid_store::domain::price_components::{PriceEntry, ProductRef};
    use meteroid_store::services::PendingMaterialization;

    let mut processed_add_ons = Vec::new();
    let mut pending_materializations = Vec::new();

    for cs_ao in &create_add_ons.add_ons {
        let add_on = add_ons
            .iter()
            .find(|x| x.id == cs_ao.add_on_id)
            .ok_or_else(|| Status::not_found(format!("Add-on {} not found", cs_ao.add_on_id)))?;

        let resolved = add_on
            .resolve_customized(products, prices, units, &cs_ao.customization)
            .map_err(|e| Status::internal(format!("Failed to resolve add-on fee: {e}")))?;

        let idx = processed_add_ons.len();

        if resolved.price_id.is_none()
            && let Some(PriceEntry::New(_)) = &resolved.price_entry
        {
            pending_materializations.push(PendingMaterialization {
                component_index: idx,
                name: resolved.name.clone(),
                product_ref: ProductRef::Existing(add_on.product_id),
                price_entry: resolved.price_entry.clone().unwrap(),
                product_family_id,
                currency: currency.to_string(),
            });
        }

        processed_add_ons.push(QuoteAddOnNew {
            quote_id,
            add_on_id: add_on.id,
            name: resolved.name,
            period: resolved.period,
            fee: resolved.fee,
            product_id: resolved.product_id,
            price_id: resolved.price_id,
            quantity: cs_ao.quantity,
        });
    }

    Ok((processed_add_ons, pending_materializations))
}
