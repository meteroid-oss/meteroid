use super::{QuoteServiceComponents, mapping};
use crate::entitlements::mapping::entitlement_spec_from_proto;
use crate::quotes::error::QuoteApiError;
use common_domain::ids::QuoteDraftId;
use common_domain::ids::{AddOnId, BaseId, CouponId, CustomerId, PlanVersionId, PriceId, QuoteId};
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::conversions::FromProtoOpt;
use meteroid_app::features::{
    QUOTE_MULTIPLE_SIGNERS_NOT_IN_PLAN, ensure_change_allowed, ensure_credit_pricing_allowed,
    gates, minimum_commitments_open, minimum_commitments_refused,
};
use meteroid_grpc::meteroid::api::quotes::v1::{
    CancelQuoteRequest, CancelQuoteResponse, ConvertQuoteToSubscriptionRequest,
    ConvertQuoteToSubscriptionResponse, CreateQuote, CreateQuoteRequest, CreateQuoteResponse,
    DuplicateQuoteRequest, DuplicateQuoteResponse, ExpireQuoteRequest, ExpireQuoteResponse,
    GenerateQuotePortalTokenRequest, GenerateQuotePortalTokenResponse, GetQuoteRequest,
    GetQuoteResponse, ListQuotesRequest, ListQuotesResponse, PreviewQuotePortalRequest,
    PreviewQuotePortalResponse, PreviewQuoteRequest, PreviewQuoteResponse,
    PreviewQuoteTotalsRequest, PreviewQuoteTotalsResponse, PublishQuoteRequest,
    PublishQuoteResponse, SendQuoteRequest, SendQuoteResponse, UpdateQuoteRequest,
    UpdateQuoteResponse, quotes_service_server::QuotesService,
};
use meteroid_grpc::meteroid::api::quotes::v1::{
    CountersignQuoteRequest, CountersignQuoteResponse, DeclineQuoteRequest, DeclineQuoteResponse,
    DeleteQuoteDraftRequest, DeleteQuoteDraftResponse, GetQuoteApprovalRequest,
    GetQuoteApprovalResponse, GetQuoteDraftRequest, GetQuoteDraftResponse, ListQuoteDraftsRequest,
    ListQuoteDraftsResponse, ListQuoteEvidenceRequest, ListQuoteEvidenceResponse,
    ListQuoteRevisionsRequest, ListQuoteRevisionsResponse, SaveQuoteDraftRequest,
    SaveQuoteDraftResponse,
};
use meteroid_grpc_mapping::utils::PaginationExt;
use meteroid_store::domain::add_ons::AddOn;
use meteroid_store::domain::customer_emails::EmailKind;
use meteroid_store::domain::price_cells::{OptionPricing, resolve_option, sold_options};
use meteroid_store::domain::quote_drafts::{QuoteDraftSave, QuoteDraftSaveOutcome};
use meteroid_store::domain::quotes::{QuoteAddOnNew, QuoteCouponNew, UnsavedQuote};
use meteroid_store::domain::{CreateSubscriptionAddOns, CreateSubscriptionComponents};
use meteroid_store::domain::{PriceComponent, quotes::QuotePriceComponentNew};
use meteroid_store::repositories::CreditUnitsInterface;
use meteroid_store::repositories::PlanOptionInterface;
use meteroid_store::repositories::QuotesInterface;
use meteroid_store::repositories::customer_emails::CustomerEmailsInterface;
use meteroid_store::repositories::portal_tokens::PortalTokensInterface;
use meteroid_store::repositories::quote_approvals::QuoteApprovalsInterface;
use meteroid_store::repositories::quote_drafts::QuoteDraftsInterface;
use meteroid_store::repositories::quote_signing::{QuoteSigningInterface, SignQuoteInput};
use nanoid::nanoid;
use tonic::{Request, Response, Status};

use crate::subscriptions::mapping::add_ons::create_subscription_add_ons_from_grpc;
use crate::subscriptions::mapping::price_components::create_subscription_components_from_grpc;
use common_utils::rng::UPPER_ALPHANUMERIC;
use meteroid_store::repositories::add_ons::AddOnInterfaceAuto;
use meteroid_store::repositories::price_components::PriceComponentInterfaceAuto;
use meteroid_store::repositories::prices::PriceInterface;

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
        let mut prepared = self
            .prepare_quote(
                tenant_id,
                quote_id,
                quote,
                format!("Q-{}", nanoid!(8, &UPPER_ALPHANUMERIC)),
            )
            .await?;
        if let common_domain::actor::Actor::User { id } = &actor {
            prepared.quote_new.owner_id = Some(*id);
        }
        super::v2::ensure_v2_allowed(
            self.feature_resolver.as_ref(),
            organization_id,
            &prepared.quote_new,
            &prepared.components,
            &prepared.add_ons,
            None,
        )
        .await?;
        if prepared.minimum.is_some()
            && !minimum_commitments_open(self.feature_resolver.as_ref(), organization_id).await
        {
            return Err(minimum_commitments_refused());
        }
        self.ensure_signers_allowed(organization_id, prepared.quote_new.recipients.len(), 1)
            .await?;
        ensure_credit_pricing_allowed(
            self.feature_resolver.as_ref(),
            organization_id,
            prepared.creates_credit_product,
        )
        .await?;

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
        self.after_quote_write(tenant_id, created_quote.id).await?;

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
        let organization_id = request.organization()?;
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
        if detailed_quote_domain.quote.signature_level
            == meteroid_store::domain::enums::QuoteSignatureLevelEnum::Verified
        {
            // The rule the portal applies when a signer asks for a code.
            let signers: Vec<String> = detailed_quote_domain
                .quote
                .recipients
                .iter()
                .map(|r| r.email.clone())
                .chain(
                    detailed_quote_domain
                        .quote
                        .countersigners
                        .iter()
                        .filter(|c| !c.automatic)
                        .map(|c| c.email.clone()),
                )
                .collect();
            detailed_quote.signature_codes_unavailable = self
                .store
                .signature_code_channels(tenant_id, &signers)
                .await
                .map_err(Into::<QuoteApiError>::into)?
                .contains(
                    &meteroid_store::domain::quote_signing::SignatureCodeChannel::Unavailable,
                );
        }
        // Display-only: the quote still loads when its totals cannot be computed.
        detailed_quote.totals = match self
            .services
            .quote_totals(tenant_id, &detailed_quote_domain)
            .await
        {
            Ok(totals) => Some(mapping::quotes::totals_to_proto(&totals)),
            Err(error) => {
                log::warn!("no totals for quote {quote_id:?}: {error:?}");
                None
            }
        };
        if self.gate_open(organization_id, gates::QUOTES_FULL).await {
            let engagement = self
                .store
                .list_quote_recipient_engagement(tenant_id, quote_id)
                .await
                .map_err(Into::<QuoteApiError>::into)?;
            let sessions = self
                .store
                .list_quote_view_sessions(tenant_id, quote_id)
                .await
                .map_err(Into::<QuoteApiError>::into)?;
            detailed_quote.engagement =
                Some(mapping::quotes::engagement_to_proto(&engagement, &sessions));
        }
        detailed_quote.pdf_url = self
            .quote_documents
            .pdf_url(tenant_id, &detailed_quote_domain.quote)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        if let Some(quote) = detailed_quote.quote.as_mut() {
            super::link_attachments(
                self.quote_documents.as_ref(),
                tenant_id,
                detailed_quote_domain.quote.id,
                quote,
            )?;
        }

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
        let organization_id = request.organization()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let quote_id = QuoteId::from_proto(&inner.id)?;

        self.ensure_approved(tenant_id, quote_id).await?;
        // Send the quote (publishes if draft, queues email)
        let _updated_quote = self
            .store
            .send_quote(
                actor.clone(),
                quote_id,
                tenant_id,
                inner.recipient_email.filter(|e| !e.trim().is_empty()),
                inner.message,
            )
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        self.after_publish(tenant_id, organization_id, quote_id, &actor)
            .await?;

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
        self.store
            .sync_quote_to_hubspot(tenant_id, quote_id)
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
        let expected_updated_at = inner
            .expected_updated_at
            .map(|at| {
                (!at.is_empty())
                    .then(|| {
                        <chrono::NaiveDateTime as meteroid_app::conversions::ProtoConv<
                                String,
                            >>::from_proto_ref(&at)
                    })
                    .transpose()
            })
            .transpose()?;

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
            .prepare_quote(tenant_id, quote_id, quote, existing.quote_number.clone())
            .await?;
        let stored = self
            .store
            .get_detailed_quote_by_id(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        let mut prepared = prepared;
        prepared.quote_new.owner_id = existing.owner_id;
        super::v2::ensure_v2_allowed(
            self.feature_resolver.as_ref(),
            organization_id,
            &prepared.quote_new,
            &prepared.components,
            &prepared.add_ons,
            Some(&stored),
        )
        .await?;
        let revisions = self.gate_open(organization_id, gates::QUOTES_FULL).await;
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
        self.ensure_signers_allowed(
            organization_id,
            prepared.quote_new.recipients.len(),
            existing.recipients.len(),
        )
        .await?;
        ensure_credit_pricing_allowed(
            self.feature_resolver.as_ref(),
            organization_id,
            prepared.creates_credit_product,
        )
        .await?;

        let replaced = self
            .store
            .replace_quote(
                prepared.quote_new,
                prepared.components,
                prepared.add_ons,
                prepared.coupons,
                prepared.pending_addon_materializations,
                prepared.minimum,
                actor.clone(),
                revisions,
                expected_updated_at,
            )
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        // A revision held for approval is recorded when it is published.
        if revisions && replaced.status == meteroid_store::domain::enums::QuoteStatusEnum::Pending {
            self.store
                .record_quote_revision(tenant_id, quote_id, Some(actor_label(&actor)))
                .await
                .map_err(Into::<QuoteApiError>::into)?;
        }
        self.after_quote_write(tenant_id, quote_id).await?;

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
    async fn preview_quote_portal(
        &self,
        request: Request<PreviewQuotePortalRequest>,
    ) -> Result<Response<PreviewQuotePortalResponse>, Status> {
        let tenant_id = request.tenant()?;
        let inner = request.into_inner();
        let quote_id = QuoteId::from_proto(&inner.quote_id)?;

        let detailed = self
            .store
            .get_detailed_quote_by_id(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        let quote = &detailed.quote;
        let viewer_email = match inner.recipient_email.as_deref().map(str::trim) {
            None | Some("") => quote
                .recipients
                .first()
                .map(|r| r.email.clone())
                .ok_or_else(|| Status::invalid_argument("The quote has no recipient"))?,
            Some(requested) => quote
                .recipients
                .iter()
                .map(|r| &r.email)
                .chain(
                    quote
                        .countersigners
                        .iter()
                        .filter(|c| !c.automatic)
                        .map(|c| &c.email),
                )
                .find(|email| email.trim().eq_ignore_ascii_case(requested))
                .cloned()
                .ok_or_else(|| {
                    Status::invalid_argument("Recipient not found in quote recipients")
                })?,
        };

        let details = crate::portal::quotes::quote_portal_details(
            &self.store,
            &self.services,
            &self.links,
            self.quote_documents.as_ref(),
            self.feature_resolver.as_ref(),
            tenant_id,
            detailed,
            viewer_email,
        )
        .await?;
        Ok(Response::new(PreviewQuotePortalResponse {
            quote: Some(details),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn preview_quote_totals(
        &self,
        request: Request<PreviewQuoteTotalsRequest>,
    ) -> Result<Response<PreviewQuoteTotalsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let quote = request
            .into_inner()
            .quote
            .ok_or_else(|| Status::invalid_argument("quote is required"))?;

        // Prepared like a create, so the totals match what saving it would store.
        let prepared = self
            .prepare_quote(tenant_id, QuoteId::new(), quote, String::new())
            .await?;
        let totals = self
            .services
            .unsaved_quote_totals(
                tenant_id,
                UnsavedQuote {
                    quote: prepared.quote_new,
                    components: prepared.components,
                    add_ons: prepared.add_ons,
                    coupons: prepared.coupons,
                    minimum: prepared.minimum,
                },
            )
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        Ok(Response::new(PreviewQuoteTotalsResponse {
            totals: Some(mapping::quotes::totals_to_proto(&totals)),
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

        // The token carries the email as stored on the quote, so every link of a person shares
        // its signing codes and evidence whatever case was typed.
        let requested = inner.recipient_email.trim();
        let recipient_email = quote
            .recipients
            .iter()
            .map(|r| &r.email)
            .chain(
                quote
                    .countersigners
                    .iter()
                    .filter(|c| !c.automatic)
                    .map(|c| &c.email),
            )
            .find(|email| email.trim().eq_ignore_ascii_case(requested))
            .cloned()
            .ok_or_else(|| Status::invalid_argument("Recipient not found in quote recipients"))?;

        let token = self
            .store
            .quote_portal_token(tenant_id, quote_id, recipient_email, delegated)
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        Ok(Response::new(GenerateQuotePortalTokenResponse { token }))
    }

    #[tracing::instrument(skip_all)]
    async fn publish_quote(
        &self,
        request: Request<PublishQuoteRequest>,
    ) -> Result<Response<PublishQuoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let organization_id = request.organization()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();

        let quote_id = QuoteId::from_proto(&inner.id)?;

        self.ensure_approved(tenant_id, quote_id).await?;
        let updated_quote = self
            .store
            .publish_quote(actor.clone(), quote_id, tenant_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        self.after_publish(tenant_id, organization_id, quote_id, &actor)
            .await?;

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

    #[tracing::instrument(skip_all)]
    async fn save_quote_draft(
        &self,
        request: Request<SaveQuoteDraftRequest>,
    ) -> Result<Response<SaveQuoteDraftResponse>, Status> {
        let tenant_id = request.tenant()?;
        let user_id = editor(&request.actor_typed()?)?;
        let inner = request.into_inner();
        let payload: serde_json::Value = serde_json::from_str(&inner.payload)
            .map_err(|e| Status::invalid_argument(format!("Invalid draft payload: {e}")))?;

        let outcome = self
            .store
            .save_quote_draft(
                tenant_id,
                user_id,
                QuoteDraftSave {
                    id: inner
                        .id
                        .as_deref()
                        .map(QuoteDraftId::from_proto)
                        .transpose()?,
                    quote_id: inner
                        .quote_id
                        .as_deref()
                        .map(QuoteId::from_proto)
                        .transpose()?,
                    expected_version: inner.expected_version,
                    label: inner.label.filter(|l| !l.trim().is_empty()),
                    payload,
                    payload_version: inner.payload_version,
                },
            )
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        let (draft, conflict) = match outcome {
            QuoteDraftSaveOutcome::Saved(d) => (d, false),
            QuoteDraftSaveOutcome::Conflict(d) => (d, true),
        };
        Ok(Response::new(SaveQuoteDraftResponse {
            draft: Some(mapping::quotes::draft_to_proto(&draft)),
            conflict,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_quote_draft(
        &self,
        request: Request<GetQuoteDraftRequest>,
    ) -> Result<Response<GetQuoteDraftResponse>, Status> {
        use meteroid_grpc::meteroid::api::quotes::v1::get_quote_draft_request::Target;

        let tenant_id = request.tenant()?;
        let draft = match request.into_inner().target {
            Some(Target::Id(id)) => {
                self.store
                    .get_quote_draft(tenant_id, QuoteDraftId::from_proto(&id)?)
                    .await
            }
            Some(Target::QuoteId(id)) => {
                self.store
                    .get_quote_draft_by_quote(tenant_id, QuoteId::from_proto(&id)?)
                    .await
            }
            None => return Err(Status::invalid_argument("id or quote_id is required")),
        }
        .map_err(Into::<QuoteApiError>::into)?;
        Ok(Response::new(GetQuoteDraftResponse {
            draft: draft.as_ref().map(mapping::quotes::draft_to_proto),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_quote_drafts(
        &self,
        request: Request<ListQuoteDraftsRequest>,
    ) -> Result<Response<ListQuoteDraftsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let drafts = self
            .store
            .list_unsaved_quote_drafts(tenant_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        Ok(Response::new(ListQuoteDraftsResponse {
            drafts: drafts.iter().map(mapping::quotes::draft_to_proto).collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn delete_quote_draft(
        &self,
        request: Request<DeleteQuoteDraftRequest>,
    ) -> Result<Response<DeleteQuoteDraftResponse>, Status> {
        let tenant_id = request.tenant()?;
        let id = QuoteDraftId::from_proto(&request.into_inner().id)?;
        self.store
            .delete_quote_draft(tenant_id, id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        Ok(Response::new(DeleteQuoteDraftResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn list_quote_revisions(
        &self,
        request: Request<ListQuoteRevisionsRequest>,
    ) -> Result<Response<ListQuoteRevisionsResponse>, Status> {
        let tenant_id = request.tenant()?;
        let quote_id = QuoteId::from_proto(&request.into_inner().quote_id)?;
        let revisions = self
            .store
            .list_quote_revisions(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        Ok(Response::new(ListQuoteRevisionsResponse {
            revisions: revisions
                .iter()
                .map(mapping::quotes::revision_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn list_quote_evidence(
        &self,
        request: Request<ListQuoteEvidenceRequest>,
    ) -> Result<Response<ListQuoteEvidenceResponse>, Status> {
        let tenant_id = request.tenant()?;
        let quote_id = QuoteId::from_proto(&request.into_inner().quote_id)?;
        let events = self
            .store
            .list_quote_evidence(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        Ok(Response::new(ListQuoteEvidenceResponse {
            chain_intact: meteroid_store::domain::quote_signing::evidence_chain_intact(&events),
            events: events
                .iter()
                .map(mapping::quotes::evidence_to_proto)
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn get_quote_approval(
        &self,
        request: Request<GetQuoteApprovalRequest>,
    ) -> Result<Response<GetQuoteApprovalResponse>, Status> {
        let tenant_id = request.tenant()?;
        let quote_id = QuoteId::from_proto(&request.into_inner().quote_id)?;
        let (quote, requirement) = self
            .store
            .quote_approval_requirement(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        Ok(Response::new(GetQuoteApprovalResponse {
            satisfied: requirement.is_satisfied(quote.document_hash.as_deref()),
            matched_rules: requirement
                .matched_rules
                .iter()
                .map(mapping::quotes::approval_rule_to_proto)
                .collect(),
            latest: requirement
                .latest
                .as_ref()
                .map(mapping::quotes::approval_to_proto),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn countersign_quote(
        &self,
        request: Request<CountersignQuoteRequest>,
    ) -> Result<Response<CountersignQuoteResponse>, Status> {
        use meteroid_store::repositories::users::UserInterface;

        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let user_id = editor(&actor)?;
        let origin = crate::portal::quotes::origin_of(&request);
        let inner = request.into_inner();
        let quote_id = QuoteId::from_proto(&inner.quote_id)?;

        let user = self
            .store
            .find_user_by_id_and_tenant(user_id, tenant_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        let quote = self
            .store
            .get_quote_by_id(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        let countersigner = quote
            .countersigners
            .iter()
            .find(|c| !c.automatic && c.email.eq_ignore_ascii_case(&user.email))
            .ok_or_else(|| {
                Status::permission_denied("You are not a countersigner of this quote")
            })?;

        self.store
            .sign_quote(
                tenant_id,
                SignQuoteInput {
                    quote_id,
                    role: meteroid_store::domain::enums::QuoteSignerRoleEnum::Countersigner,
                    signed_by_name: countersigner.name.clone(),
                    signed_by_email: countersigner.email.clone(),
                    signed_by_title: inner
                        .title
                        .filter(|t| !t.trim().is_empty())
                        .or_else(|| countersigner.title.clone()),
                    signature_data: inner.signature_data.filter(|d| !d.is_empty()),
                    signature_method: "countersignature".to_string(),
                    document_hash: Some(inner.document_hash),
                    consent_text: inner.consent_text,
                    authenticated: true,
                    origin,
                    actor,
                },
            )
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        self.store
            .sync_quote_to_hubspot(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;

        let detailed = self
            .store
            .get_detailed_quote_by_id(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        Ok(Response::new(CountersignQuoteResponse {
            quote: Some(mapping::quotes::detailed_quote_domain_to_proto(&detailed)),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn decline_quote(
        &self,
        request: Request<DeclineQuoteRequest>,
    ) -> Result<Response<DeclineQuoteResponse>, Status> {
        let tenant_id = request.tenant()?;
        let actor = request.actor_typed()?;
        let inner = request.into_inner();
        let quote_id = QuoteId::from_proto(&inner.id)?;

        let quote = self
            .store
            .get_quote_by_id(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        if quote.status != meteroid_store::domain::enums::QuoteStatusEnum::Pending {
            return Err(Status::failed_precondition(
                "Only pending quotes can be declined",
            ));
        }
        self.store
            .decline_quote(
                actor,
                quote_id,
                tenant_id,
                inner.reason.filter(|r| !r.trim().is_empty()),
            )
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        self.store
            .sync_quote_to_hubspot(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        let detailed = self
            .store
            .get_detailed_quote_by_id(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        Ok(Response::new(DeclineQuoteResponse {
            quote: Some(mapping::quotes::detailed_quote_domain_to_proto(&detailed)),
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

/// Drafts and countersignatures belong to a signed-in member, not an API token.
fn editor(actor: &common_domain::actor::Actor) -> Result<common_domain::ids::UserId, Status> {
    match actor {
        common_domain::actor::Actor::User { id } => Ok(*id),
        _ => Err(Status::permission_denied(
            "Only a signed-in member can do this",
        )),
    }
}

fn actor_label(actor: &common_domain::actor::Actor) -> String {
    use common_domain::actor::Actor;
    match actor {
        Actor::User { id } => format!("user:{}", id.as_uuid()),
        Actor::ApiToken { id } => format!("api_token:{}", id.as_uuid()),
        other => format!("{:?}", other.actor_type()).to_lowercase(),
    }
}

impl QuoteServiceComponents {
    async fn gate_open(
        &self,
        organization_id: common_domain::ids::OrganizationId,
        code: meteroid_app::features::FeatureCode,
    ) -> bool {
        self.feature_resolver
            .resolve(organization_id)
            .await
            .is_enabled(code)
    }

    async fn resolve_attachments(
        &self,
        tenant_id: common_domain::ids::TenantId,
        ids: Vec<String>,
    ) -> Result<Vec<meteroid_store::domain::quotes::QuoteAttachment>, Status> {
        let ids = ids
            .iter()
            .map(common_domain::ids::QuoteAttachmentId::from_proto)
            .collect::<Result<Vec<_>, _>>()?;
        self.quote_documents
            .resolve_attachments(tenant_id, &ids)
            .await
            .map_err(|e| Into::<QuoteApiError>::into(e).into())
    }

    /// Recomputes what the edit changed: the document hash, the largest discount, the
    /// approval status, and the linked deal.
    async fn after_quote_write(
        &self,
        tenant_id: common_domain::ids::TenantId,
        quote_id: QuoteId,
    ) -> Result<(), Status> {
        self.store
            .refresh_quote_document(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        self.store
            .quote_approval_requirement(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        self.store
            .sync_quote_to_hubspot(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        Ok(())
    }

    async fn after_publish(
        &self,
        tenant_id: common_domain::ids::TenantId,
        organization_id: common_domain::ids::OrganizationId,
        quote_id: QuoteId,
        actor: &common_domain::actor::Actor,
    ) -> Result<(), Status> {
        if self.gate_open(organization_id, gates::QUOTES_FULL).await {
            self.store
                .record_quote_revision(tenant_id, quote_id, Some(actor_label(actor)))
                .await
                .map_err(Into::<QuoteApiError>::into)?;
        }
        self.store
            .sync_quote_to_hubspot(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        Ok(())
    }

    /// Publishing a quote that matches an approval rule needs an approval of its current content.
    async fn ensure_approved(
        &self,
        tenant_id: common_domain::ids::TenantId,
        quote_id: QuoteId,
    ) -> Result<(), Status> {
        let quote = self
            .store
            .get_quote_by_id(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        if quote.status != meteroid_store::domain::enums::QuoteStatusEnum::Draft {
            return Ok(());
        }
        self.store
            .refresh_quote_document(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        let (quote, requirement) = self
            .store
            .quote_approval_requirement(tenant_id, quote_id)
            .await
            .map_err(Into::<QuoteApiError>::into)?;
        if requirement.is_satisfied(quote.document_hash.as_deref()) {
            return Ok(());
        }
        let rules = requirement
            .matched_rules
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        Err(Status::failed_precondition(format!(
            "This quote needs an approval before it is published ({rules})"
        )))
    }

    async fn ensure_signers_allowed(
        &self,
        organization_id: common_domain::ids::OrganizationId,
        signers: usize,
        stored: usize,
    ) -> Result<(), Status> {
        ensure_change_allowed(
            self.feature_resolver.as_ref(),
            organization_id,
            gates::QUOTES_FULL,
            signers > stored.max(1),
            QUOTE_MULTIPLE_SIGNERS_NOT_IN_PLAN,
        )
        .await
    }
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
    creates_credit_product: bool,
}

impl QuoteServiceComponents {
    async fn prepare_quote(
        &self,
        tenant_id: common_domain::ids::TenantId,
        quote_id: QuoteId,
        mut quote: CreateQuote,
        default_quote_number: String,
    ) -> Result<PreparedQuote, Status> {
        let attachments = self
            .resolve_attachments(tenant_id, std::mem::take(&mut quote.attachments))
            .await?;
        let line_details = std::mem::take(&mut quote.line_details);
        let display_options =
            mapping::quotes::display_options_to_domain(quote.display_options.take());
        let sections = std::mem::take(&mut quote.sections)
            .into_iter()
            .map(mapping::quotes::section_to_domain)
            .collect();
        let countersigners = std::mem::take(&mut quote.countersigners)
            .into_iter()
            .map(mapping::quotes::countersigner_to_domain)
            .collect();
        let signature_level = mapping::quotes::signature_level_to_domain(quote.signature_level());
        let hubspot_deal_id = quote
            .hubspot_deal_id
            .take()
            .map(|d| super::v2::parse_hubspot_deal_id(&d))
            .transpose()?
            .flatten();
        let template_id = quote
            .template_id
            .take()
            .filter(|t| !t.is_empty())
            .map(|t| common_domain::ids::QuoteTemplateId::from_proto(&t))
            .transpose()?;
        let consent_text = quote.consent_text.take().filter(|c| !c.trim().is_empty());
        let customer_actions = quote.customer_actions.take();
        let customer_id = CustomerId::from_proto(&quote.customer_id)?;
        let plan_version_id = PlanVersionId::from_proto(&quote.plan_version_id)?;
        let asked_option = crate::plans::mapping::plans::option_ref_from_proto(quote.option.take());

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
            // Set once the components are priced in an option, below.
            plan_option: None,
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
            attachments,
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
            owner_id: None,
            template_id,
            display_options,
            sections,
            signature_level,
            consent_text,
            countersigners,
            hubspot_deal_id,
            max_discount_percent: None,
            allow_questions: customer_actions.as_ref().is_none_or(|a| a.questions),
            allow_change_requests: customer_actions.as_ref().is_none_or(|a| a.change_requests),
            allow_decline: customer_actions.as_ref().is_none_or(|a| a.decline),
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

        let mut creates_credit_product = false;
        // Process quote components (fetch plan price components + products + prices first)
        let mut quote_components = if let Some(components) = quote.components {
            // A quote is priced in its currency, as its subscription will be.
            let mut price_components = self
                .store
                .list_price_components(plan_version_id, tenant_id)
                .await
                .map_err(Into::<QuoteApiError>::into)?;
            let create_components = create_subscription_components_from_grpc(components)?;
            // Priced in one option of the plan: the one asked, else the first sold in the currency.
            let options = self
                .store
                .list_plan_options(tenant_id, plan_version_id)
                .await
                .map_err(Into::<QuoteApiError>::into)?;
            let option = match &asked_option {
                Some(asked) => resolve_option(&options, Some(asked)),
                None => {
                    match sold_options(&options, &price_components, &quote_new.currency).first() {
                        Some(option) => Ok(*option),
                        None => resolve_option(&options, None),
                    }
                }
            }
            .map_err(|e| Status::invalid_argument(e.current_context().to_string()))?;
            quote_new.plan_option = Some(option.alias.clone());
            let pricing = OptionPricing::new(option.id, &quote_new.currency);
            // As a subscription's creation does: a removed or overridden component sells nothing.
            for component in &mut price_components {
                let replaced = create_components.remove_components.contains(&component.id)
                    || create_components
                        .overridden_components
                        .iter()
                        .any(|o| o.component_id == component.id);
                if replaced {
                    continue;
                }
                // Never taken for an included product only for lack of a price in the cell.
                if !component.prices.is_empty() && pricing.prices(component).is_empty() {
                    return Err(Status::invalid_argument(format!(
                        "{} has no price in {} for {}",
                        component.name, quote_new.currency, option.name
                    )));
                }
                pricing.narrow(std::slice::from_mut(component));
            }
            creates_credit_product = create_components.extra_components.iter().any(|e| {
                e.fee_structure
                    .as_ref()
                    .or_else(|| e.product_ref.new_product_structure())
                    .is_some_and(|fs| fs.is_credit())
            });

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
        // The subscription it converts to bills in the plan's currency.
        if quote_new.currency != plan_version.currency {
            return Err(Status::invalid_argument(format!(
                "The quote is in {} but its plan bills in {}",
                quote_new.currency, plan_version.currency
            )));
        }

        // Process quote add-ons (fetch add-on details first)
        let (mut quote_add_ons, mut pending_addon_materializations) = if let Some(add_ons_proto) =
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

                // Collect price_ids from add-ons for fee resolution (each its own structure)
                let price_ids: Vec<PriceId> = add_ons.iter().flat_map(|a| a.price_ids()).collect();

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

                // Quoted on the plan's term: the shortest recurring period of its charges.
                let term = quote_components
                    .iter()
                    .filter_map(|c| c.period.as_billing_period_opt())
                    .min();
                process_quote_add_ons(
                    &create_add_ons,
                    &add_ons,
                    &prices_map,
                    &units,
                    quote_id,
                    plan_with_version.plan.product_family_id,
                    &plan_version.currency,
                    term,
                )?
            } else {
                (vec![], vec![])
            }
        } else {
            (vec![], vec![])
        };

        super::v2::apply_line_details(
            &line_details,
            &mut quote_components,
            &mut quote_add_ons,
            &mut pending_addon_materializations,
            &quote_new.currency,
        )?;

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
            creates_credit_product,
        })
    }
}

fn process_quote_components(
    components: &CreateSubscriptionComponents,
    price_components: &[PriceComponent],
    prices: &std::collections::HashMap<PriceId, meteroid_store::domain::Price>,
    units: &meteroid_store::domain::credit_units::CreditUnitsById,
    quote_id: QuoteId,
) -> Result<Vec<QuotePriceComponentNew>, Status> {
    use meteroid_store::domain::price_components::{ComponentParameters, PriceEntry, ProductRef};
    use meteroid_store::domain::prices::resolve_fee_from_entry;

    let mut processed_components = Vec::new();

    // Process parameterized components. An overridden one is priced by its override below,
    // which takes the parameters (the seat count), as a subscription's creation does.
    for parameterized in &components.parameterized_components {
        if components
            .overridden_components
            .iter()
            .any(|o| o.component_id == parameterized.component_id)
        {
            continue;
        }
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
                .resolve_subscription_fee(units, Some(&params))
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
                description: None,
                sort_order: 0,
                discount_percent: None,
                list_fee: None,
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

            let fee_structure = price_component.structure().ok_or_else(|| {
                Status::invalid_argument(format!(
                    "Cannot override component {}: it includes its product with no charge",
                    price_component.id
                ))
            })?;

            let (mut fee, period) =
                resolve_fee_from_entry(fee_structure, &overridden.price_entry, prices, units)
                    .map_err(|e| {
                        Status::internal(format!("Failed to resolve override fee: {e}"))
                    })?;
            if let Some(parameterized) = components
                .parameterized_components
                .iter()
                .find(|p| p.component_id == overridden.component_id)
            {
                fee.apply_parameters(&parameterized.parameters);
            }

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
                description: None,
                sort_order: 0,
                discount_percent: None,
                list_fee: None,
            });
        }
    }

    // Process extra components
    for extra in &components.extra_components {
        // The charge's own shape, else its new product's, else that of the plan charge its
        // existing price is linked to.
        let linked = extra.price_entry.existing_price_id().and_then(|price_id| {
            price_components
                .iter()
                .find(|pc| pc.prices.iter().any(|p| p.id == price_id))
                .and_then(|pc| pc.structure().cloned())
        });
        let fee_structure = extra
            .fee_structure
            .clone()
            .or_else(|| extra.product_ref.new_product_structure().cloned())
            .or(linked)
            .ok_or_else(|| {
                Status::invalid_argument(format!(
                    "{}: name how the charge bills its product",
                    extra.name
                ))
            })?;

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
            description: None,
            sort_order: 0,
            discount_percent: None,
            list_fee: None,
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
            .resolve_subscription_fee(units, None)
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
            description: None,
            sort_order: 0,
            discount_percent: None,
            list_fee: None,
        });
    }

    Ok(processed_components)
}

fn process_quote_add_ons(
    create_add_ons: &CreateSubscriptionAddOns,
    add_ons: &[AddOn],
    prices: &std::collections::HashMap<common_domain::ids::PriceId, meteroid_store::domain::Price>,
    units: &meteroid_store::domain::credit_units::CreditUnitsById,
    quote_id: QuoteId,
    product_family_id: common_domain::ids::ProductFamilyId,
    currency: &str,
    term: Option<meteroid_store::domain::enums::BillingPeriodEnum>,
) -> Result<
    (
        Vec<QuoteAddOnNew>,
        Vec<meteroid_store::services::PendingMaterialization>,
    ),
    Status,
> {
    use meteroid_store::domain::price_components::{PriceEntry, ProductRef};
    use meteroid_store::domain::subscription_add_ons::SubscriptionAddOnCustomization;
    use meteroid_store::services::PendingMaterialization;

    let mut processed_add_ons = Vec::new();
    let mut pending_materializations = Vec::new();

    for cs_ao in &create_add_ons.add_ons {
        let add_on = add_ons
            .iter()
            .find(|x| x.id == cs_ao.add_on_id)
            .ok_or_else(|| Status::not_found(format!("Add-on {} not found", cs_ao.add_on_id)))?;
        // A seat add-on bills its quantity's count: one instance at most.
        if add_on.charges_seats() && cs_ao.quantity > 1 {
            return Err(Status::invalid_argument(format!(
                "{} charges per seat: one instance at most",
                add_on.name
            )));
        }

        // Quoted at its price in the quote's currency on its term (the one asked, else the
        // plan's); a price override keeps its own.
        let add_on = match &cs_ao.customization {
            SubscriptionAddOnCustomization::PriceOverride { .. } => add_on.clone(),
            SubscriptionAddOnCustomization::Parameterization(param) => add_on
                .in_currency(currency, param.billing_period.or(term))
                .map_err(|e| Status::invalid_argument(e.to_string()))?,
            SubscriptionAddOnCustomization::None => add_on
                .in_currency(currency, term)
                .map_err(|e| Status::invalid_argument(e.to_string()))?,
        };
        let add_on = &add_on;
        let resolved = add_on
            .resolve_customized(prices, units, &cs_ao.customization)
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
                fee_structure: add_on.fee_structure.clone(),
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
            description: None,
            sort_order: 0,
            discount_percent: None,
            list_fee: None,
        });
    }

    Ok((processed_add_ons, pending_materializations))
}
