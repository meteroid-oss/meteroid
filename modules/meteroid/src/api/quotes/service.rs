use super::{QuoteServiceComponents, mapping};
use crate::api::quotes::error::QuoteApiError;
use crate::api::shared::conversions::FromProtoOpt;
use crate::api::utils::PaginationExt;
use common_domain::ids::{AddOnId, BaseId, CouponId, CustomerId, PlanVersionId, QuoteId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::api::quotes::v1::{
    CancelQuoteRequest, CancelQuoteResponse, ConvertQuoteToSubscriptionRequest,
    ConvertQuoteToSubscriptionResponse, CreateQuoteRequest, CreateQuoteResponse,
    DuplicateQuoteRequest, DuplicateQuoteResponse, ExpireQuoteRequest, ExpireQuoteResponse,
    GenerateQuotePortalTokenRequest, GenerateQuotePortalTokenResponse, GetQuoteRequest,
    GetQuoteResponse, ListQuotesRequest, ListQuotesResponse, PreviewQuoteRequest,
    PreviewQuoteResponse, PublishQuoteRequest, PublishQuoteResponse, SendQuoteRequest,
    SendQuoteResponse, UpdateQuoteRequest, UpdateQuoteResponse, list_quotes_request::SortBy,
    quotes_service_server::QuotesService,
};
use meteroid_store::domain::add_ons::AddOn;
use meteroid_store::domain::quotes::{QuoteAddOnNew, QuoteCouponNew};
use meteroid_store::domain::{
    CreateSubscriptionAddOns, CreateSubscriptionComponents, OrderByRequest,
};
use meteroid_store::domain::{PriceComponent, quotes::QuotePriceComponentNew};
use meteroid_store::repositories::QuotesInterface;
use nanoid::nanoid;
use tonic::{Request, Response, Status};

use crate::api::subscriptions::mapping::add_ons::create_subscription_add_ons_from_grpc;
use crate::api::subscriptions::mapping::price_components::create_subscription_components_from_grpc;
use common_utils::rng::UPPER_ALPHANUMERIC;
use meteroid_store::repositories::add_ons::AddOnInterface;
use meteroid_store::repositories::price_components::PriceComponentInterface;

#[tonic::async_trait]
impl QuotesService for QuoteServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn create_quote(
        &self,
        request: Request<CreateQuoteRequest>,
    ) -> Result<Response<CreateQuoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let quote = inner
            .quote
            .ok_or_else(|| Status::invalid_argument("quote is required"))?;

        let customer_id = CustomerId::from_proto(&quote.customer_id)?;
        let plan_version_id = PlanVersionId::from_proto(&quote.plan_version_id)?;

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

        // Parse payment strategy (defaults to Auto if not provided)
        let payment_strategy = quote
            .payment_strategy
            .and_then(|ps| {
                meteroid_grpc::meteroid::api::quotes::v1::PaymentStrategy::try_from(ps).ok()
            })
            .map(mapping::quotes::payment_strategy_from_proto)
            .unwrap_or(meteroid_store::domain::enums::SubscriptionPaymentStrategy::Auto);

        let quote_id = QuoteId::new();

        let quote_new = meteroid_store::domain::quotes::QuoteNew {
            id: quote_id,
            status: meteroid_store::domain::enums::QuoteStatusEnum::Draft,
            tenant_id,
            customer_id,
            plan_version_id,
            currency: quote.currency,
            quote_number: quote
                .quote_number
                .unwrap_or_else(|| format!("Q-{}", nanoid!(8, &UPPER_ALPHANUMERIC))),
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
            valid_until: quote.valid_until.and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.naive_utc())
            }),
            expires_at: quote.expires_at.and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.naive_utc())
            }),
            internal_notes: quote.internal_notes,
            cover_image: quote.cover_image.and_then(|s| s.parse().ok()),
            overview: quote.overview,
            terms_and_services: quote.terms_and_services,
            net_terms: quote.net_terms.unwrap_or(30),
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
            payment_strategy,
            auto_advance_invoices: quote.auto_advance_invoices.unwrap_or(true),
            charge_automatically: quote.charge_automatically.unwrap_or(true),
            invoice_memo: quote.invoice_memo,
            invoice_threshold: quote
                .invoice_threshold
                .and_then(|s| s.parse::<rust_decimal::Decimal>().ok()),
            create_subscription_on_acceptance: quote
                .create_subscription_on_acceptance
                .unwrap_or(false),
        };

        // Process quote components (fetch plan price components first)
        let quote_components = if let Some(components) = quote.components {
            let price_components = self
                .store
                .list_price_components(plan_version_id, tenant_id)
                .await
                .map_err(Into::<QuoteApiError>::into)?;

            process_quote_components(
                &create_subscription_components_from_grpc(components)?,
                &price_components,
                quote_id,
            )?
        } else {
            vec![]
        };

        // Process quote add-ons (fetch add-on details first)
        let quote_add_ons = if let Some(add_ons_proto) = quote.add_ons {
            let create_add_ons = create_subscription_add_ons_from_grpc(add_ons_proto)?;

            if !create_add_ons.add_ons.is_empty() {
                let add_on_ids: Vec<AddOnId> =
                    create_add_ons.add_ons.iter().map(|a| a.add_on_id).collect();

                let add_ons = self
                    .store
                    .list_add_ons_by_ids(tenant_id, add_on_ids)
                    .await
                    .map_err(Into::<QuoteApiError>::into)?;

                process_quote_add_ons(&create_add_ons, &add_ons, quote_id)?
            } else {
                vec![]
            }
        } else {
            vec![]
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

        // Create quote with all details in a single transaction
        let created_quote = self
            .store
            .insert_quote_with_details(quote_new, quote_components, quote_add_ons, quote_coupons)
            .await
            .map_err(Into::<QuoteApiError>::into)?;

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

        let detailed_quote =
            mapping::quotes::detailed_quote_domain_to_proto(&detailed_quote_domain);

        Ok(Response::new(GetQuoteResponse {
            quote: Some(detailed_quote),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_quotes(
        &self,
        request: Request<ListQuotesRequest>,
    ) -> Result<Response<ListQuotesResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();

        let customer_id = CustomerId::from_proto_opt(inner.customer_id)?;
        let pagination_req = inner.pagination.into_domain();

        // TODO separate sort by for quote
        let order_by = match inner.sort_by.try_into() {
            Ok(SortBy::CreatedAtAsc) => OrderByRequest::IdAsc,
            Ok(SortBy::CreatedAtDesc) => OrderByRequest::IdDesc,
            Ok(SortBy::QuoteNumberAsc) => OrderByRequest::NameAsc,
            Ok(SortBy::QuoteNumberDesc) => OrderByRequest::NameDesc,
            Ok(SortBy::ExpiresAtAsc) => OrderByRequest::DateAsc,
            Ok(SortBy::ExpiresAtDesc) => OrderByRequest::DateDesc,
            Err(_) => OrderByRequest::IdDesc,
        };

        let status = mapping::quotes::status_server_to_domain(inner.status);

        let quotes = self
            .store
            .list_quotes(
                tenant_id,
                customer_id,
                status,
                inner.search,
                order_by,
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
        let inner = request.into_inner();

        let quote_id = QuoteId::from_proto(&inner.id)?;

        // Send the quote (publishes if draft, queues email)
        let _updated_quote = self
            .store
            .send_quote(quote_id, tenant_id, inner.message)
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
        let inner = request.into_inner();

        let quote_id = QuoteId::from_proto(&inner.id)?;

        // Cancel the quote
        let updated_quote = self
            .store
            .cancel_quote(quote_id, tenant_id, inner.reason)
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
        _request: Request<UpdateQuoteRequest>,
    ) -> Result<Response<UpdateQuoteResponse>, Status> {
        unimplemented!()
    }

    #[tracing::instrument(skip_all)]
    async fn generate_quote_portal_token(
        &self,
        request: Request<GenerateQuotePortalTokenRequest>,
    ) -> Result<Response<GenerateQuotePortalTokenResponse>, Status> {
        let tenant_id = request.tenant()?;
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
        let inner = request.into_inner();

        let quote_id = QuoteId::from_proto(&inner.id)?;

        let updated_quote = self
            .store
            .publish_quote(quote_id, tenant_id)
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
        let actor = request.actor()?;
        let inner = request.into_inner();

        let quote_id = QuoteId::from_proto(&inner.quote_id)?;

        // Convert the quote to a subscription
        let result = self
            .services
            .convert_quote_to_subscription(tenant_id, quote_id, actor)
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        // Get the updated quote
        let updated_quote = self
            .store
            .get_quote_by_id(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        // Map the subscription to proto
        let subscription =
            crate::api::subscriptions::mapping::subscriptions::created_domain_to_proto(
                result.subscription,
            )?;

        Ok(Response::new(ConvertQuoteToSubscriptionResponse {
            quote: Some(mapping::quotes::quote_to_proto(&updated_quote, None, false)),
            subscription: Some(subscription),
        }))
    }
}

fn process_quote_components(
    components: &CreateSubscriptionComponents,
    price_components: &[PriceComponent],
    quote_id: QuoteId,
) -> Result<Vec<QuotePriceComponentNew>, Status> {
    let mut processed_components = Vec::new();

    // Process parameterized components
    for parameterized in &components.parameterized_components {
        if let Some(price_component) = price_components
            .iter()
            .find(|pc| pc.id == parameterized.component_id)
        {
            let (period, fee) = price_component
                .fee
                .to_subscription_fee_parameterized(
                    &parameterized.parameters.initial_slot_count,
                    &parameterized.parameters.billing_period,
                    &parameterized.parameters.committed_capacity,
                )
                .map_err(|e| Status::internal(format!("Failed to process component fee: {e}")))?;

            processed_components.push(QuotePriceComponentNew {
                name: price_component.name.clone(),
                quote_id,
                price_component_id: Some(price_component.id),
                product_id: price_component.product_id,
                period,
                fee,
                is_override: false,
            });
        }
    }

    // Process overridden components
    for overridden in &components.overridden_components {
        if let Some(price_component) = price_components
            .iter()
            .find(|pc| pc.id == overridden.component_id)
        {
            processed_components.push(QuotePriceComponentNew {
                name: overridden.component.name.clone(),
                quote_id,
                price_component_id: Some(price_component.id),
                product_id: overridden.component.product_id,
                period: overridden.component.period,
                fee: overridden.component.fee.clone(),
                is_override: true,
            });
        }
    }

    // Process extra components
    for extra in &components.extra_components {
        processed_components.push(QuotePriceComponentNew {
            name: extra.component.name.clone(),
            quote_id,
            price_component_id: None,
            product_id: extra.component.product_id,
            period: extra.component.period,
            fee: extra.component.fee.clone(),
            is_override: true,
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

        let (period, fee) = price_component.fee.to_subscription_fee().map_err(|e| {
            Status::internal(format!("Failed to process default component fee: {e}"))
        })?;

        processed_components.push(QuotePriceComponentNew {
            name: price_component.name.clone(),
            quote_id,
            price_component_id: Some(price_component.id),
            product_id: price_component.product_id,
            period,
            fee,
            is_override: false,
        });
    }

    Ok(processed_components)
}

fn process_quote_add_ons(
    create_add_ons: &CreateSubscriptionAddOns,
    add_ons: &[AddOn],
    quote_id: QuoteId,
) -> Result<Vec<QuoteAddOnNew>, Status> {
    use meteroid_store::domain::SubscriptionAddOnCustomization;

    let mut processed_add_ons = Vec::new();

    for cs_ao in &create_add_ons.add_ons {
        let add_on = add_ons
            .iter()
            .find(|x| x.id == cs_ao.add_on_id)
            .ok_or_else(|| Status::not_found(format!("Add-on {} not found", cs_ao.add_on_id)))?;

        match &cs_ao.customization {
            SubscriptionAddOnCustomization::None => {
                let (period, fee) = add_on
                    .fee
                    .to_subscription_fee()
                    .map_err(|e| Status::internal(format!("Failed to process add-on fee: {e}")))?;
                processed_add_ons.push(QuoteAddOnNew {
                    quote_id,
                    add_on_id: add_on.id,
                    name: add_on.name.clone(),
                    period,
                    fee,
                });
            }
            SubscriptionAddOnCustomization::Override(override_) => {
                processed_add_ons.push(QuoteAddOnNew {
                    quote_id,
                    add_on_id: add_on.id,
                    name: override_.name.clone(),
                    period: override_.period,
                    fee: override_.fee.clone(),
                });
            }
            SubscriptionAddOnCustomization::Parameterization(param) => {
                let (period, fee) = add_on
                    .fee
                    .to_subscription_fee_parameterized(
                        &param.initial_slot_count,
                        &param.billing_period,
                        &param.committed_capacity,
                    )
                    .map_err(|e| Status::internal(format!("Failed to process add-on fee: {e}")))?;
                processed_add_ons.push(QuoteAddOnNew {
                    quote_id,
                    add_on_id: add_on.id,
                    name: add_on.name.clone(),
                    period,
                    fee,
                });
            }
        }
    }

    Ok(processed_add_ons)
}
