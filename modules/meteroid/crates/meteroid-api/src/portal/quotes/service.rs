use crate::portal::quotes::PortalQuoteServiceComponents;
use crate::portal::quotes::error::PortalQuoteApiError;
use common_domain::actor::Actor;
use common_domain::ids::TenantId;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_app::conversions::ProtoConv;
use meteroid_app::features::{
    Entitlements, QUOTE_PURCHASE_ORDERS_NOT_IN_PLAN, QUOTES_FULL_NOT_IN_PLAN, gates,
    tenant_gate_open,
};
use meteroid_app::mapping::quote_comments::comment_to_proto;
use meteroid_grpc::meteroid::portal::quotes::v1::portal_quote_service_server::PortalQuoteService;
use meteroid_grpc::meteroid::portal::quotes::v1::{
    AddQuoteCommentRequest, AddQuoteCommentResponse, DeclineQuoteRequest, DeclineQuoteResponse,
    GetQuotePortalRequest, GetQuotePortalResponse, QuoteCountersignerStatus, QuotePortalDetails,
    QuoteRecipient, QuoteSignature, SetQuotePurchaseOrderRequest, SetQuotePurchaseOrderResponse,
    SignQuoteRequest, SignQuoteResponse,
};
use meteroid_store::domain::enums::{QuoteSignerRoleEnum, QuoteStatusEnum};
use meteroid_store::domain::quote_comments::{QuoteCommentAuthor, QuoteCommentNew};
use meteroid_store::domain::quote_signing::RequestOrigin;
use meteroid_store::domain::quotes::DetailedQuote;
use meteroid_store::hooks::QuoteDocuments;
use meteroid_store::repositories::plans::PlansInterface;
use meteroid_store::repositories::quote_comments::QuoteCommentsInterface;
use meteroid_store::repositories::quote_signing::{QuoteSigningInterface, SignQuoteInput};
use meteroid_store::repositories::quotes::QuotesInterface;
use meteroid_store::{Services, Store};
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl PortalQuoteService for PortalQuoteServiceComponents {
    #[tracing::instrument(skip_all)]
    async fn get_quote_portal(
        &self,
        request: Request<GetQuotePortalRequest>,
    ) -> Result<Response<GetQuotePortalResponse>, Status> {
        let tenant = request.tenant()?;
        let quote_id = request.portal_resource()?.quote()?;

        let mut detailed_quote = self
            .store
            .get_detailed_quote_by_id(tenant, quote_id)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;
        // Quotes published before document hashes existed get theirs on first view.
        if detailed_quote.quote.document_hash.is_none()
            && detailed_quote.quote.status == QuoteStatusEnum::Pending
        {
            let document = self
                .store
                .refresh_quote_document(tenant, quote_id)
                .await
                .map_err(Into::<PortalQuoteApiError>::into)?;
            detailed_quote.quote.document_hash = Some(document.hash());
        }

        // Sent back to draft for an approval: its pending terms are not the customer's to see.
        if detailed_quote.quote.status == QuoteStatusEnum::Draft
            && detailed_quote.quote.published_at.is_some()
        {
            return Err(PortalQuoteApiError::BeingRevised.into());
        }

        // Check if quote is still valid
        if let Some(expires_at) = detailed_quote.quote.expires_at
            && expires_at < common_clock::now().naive_utc()
        {
            return Err(PortalQuoteApiError::QuoteExpired.into());
        }

        let viewer_email = request.portal_resource()?.quote_recipient_email()?;
        let details = quote_portal_details(
            &self.store,
            &self.services,
            &self.links,
            self.quote_documents.as_ref(),
            self.feature_resolver.as_ref(),
            tenant,
            detailed_quote,
            viewer_email.clone(),
        )
        .await?;
        self.store
            .record_quote_view(tenant, quote_id, viewer_email, origin_of(&request))
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;

        Ok(Response::new(GetQuotePortalResponse {
            quote: Some(details),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn sign_quote(
        &self,
        request: Request<SignQuoteRequest>,
    ) -> Result<Response<SignQuoteResponse>, Status> {
        let tenant = request.tenant()?;
        let quote_id = request.portal_resource()?.quote()?;
        let token_recipient_email = request.portal_resource()?.quote_recipient_email()?;
        let origin = origin_of(&request);
        let inner = request.into_inner();

        if !inner
            .recipient_email
            .eq_ignore_ascii_case(&token_recipient_email)
        {
            return Err(PortalQuoteApiError::InvalidArgument(
                "Signing email must match the token recipient email".to_string(),
            )
            .into());
        }
        if !PORTAL_SIGNATURE_METHODS.contains(&inner.signature_method.as_str()) {
            return Err(PortalQuoteApiError::InvalidArgument(
                "Sign by drawing or typing your signature".to_string(),
            )
            .into());
        }

        let quote = self
            .store
            .get_quote_by_id(tenant, quote_id)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;
        let role = if quote
            .recipients
            .iter()
            .any(|r| r.email.eq_ignore_ascii_case(&token_recipient_email))
        {
            QuoteSignerRoleEnum::Customer
        } else {
            QuoteSignerRoleEnum::Countersigner
        };

        let outcome = self
            .store
            .sign_quote(
                tenant,
                SignQuoteInput {
                    quote_id,
                    role,
                    signed_by_name: inner.signed_by_name,
                    signed_by_email: token_recipient_email.clone(),
                    signed_by_title: Some(inner.signed_by_title).filter(|t| !t.trim().is_empty()),
                    signature_data: Some(inner.signature_data).filter(|d| !d.is_empty()),
                    signature_method: inner.signature_method,
                    document_hash: inner.document_hash,
                    consent_text: inner.consent_text,
                    authenticated: false,
                    origin,
                    actor: Actor::QuoteRecipient {
                        email: token_recipient_email,
                    },
                },
            )
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;

        if outcome.awaiting_countersignature && role == QuoteSignerRoleEnum::Customer {
            self.store
                .notify_quote_countersigners(tenant, quote_id)
                .await
                .map_err(Into::<PortalQuoteApiError>::into)?;
        }
        self.store
            .sync_quote_to_hubspot(tenant, quote_id)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;

        Ok(Response::new(SignQuoteResponse {
            success: true,
            signature_id: outcome.signature.id.as_proto(),
            accepted: outcome.accepted,
            awaiting_countersignature: outcome.awaiting_countersignature,
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn decline_quote(
        &self,
        request: Request<DeclineQuoteRequest>,
    ) -> Result<Response<DeclineQuoteResponse>, Status> {
        let tenant = request.tenant()?;
        let quote_id = request.portal_resource()?.quote()?;
        let email = request.portal_resource()?.quote_recipient_email()?;
        let reason = request.into_inner().reason.filter(|r| !r.trim().is_empty());

        let quote = self
            .store
            .get_quote_by_id(tenant, quote_id)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;
        if quote.status != QuoteStatusEnum::Pending {
            return Err(PortalQuoteApiError::NotSignable.into());
        }
        if !quote
            .recipients
            .iter()
            .any(|r| r.email.eq_ignore_ascii_case(&email))
        {
            return Err(PortalQuoteApiError::RecipientNotFound.into());
        }
        if !quote.allow_decline {
            return Err(PortalQuoteApiError::FailedPrecondition(
                "This quote cannot be declined online".to_string(),
            )
            .into());
        }

        self.store
            .decline_quote(Actor::QuoteRecipient { email }, quote_id, tenant, reason)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;
        self.store
            .sync_quote_to_hubspot(tenant, quote_id)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;
        Ok(Response::new(DeclineQuoteResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn set_quote_purchase_order(
        &self,
        request: Request<SetQuotePurchaseOrderRequest>,
    ) -> Result<Response<SetQuotePurchaseOrderResponse>, Status> {
        let tenant = request.tenant()?;
        let quote_id = request.portal_resource()?.quote()?;
        let email = request.portal_resource()?.quote_recipient_email()?;

        // Get quote to verify status
        let quote = self
            .store
            .get_quote_by_id(tenant, quote_id)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;

        // Check if quote is in editable state
        if quote.status != QuoteStatusEnum::Pending {
            return Err(PortalQuoteApiError::NotEditable.into());
        }
        if !quote
            .recipients
            .iter()
            .any(|r| r.email.eq_ignore_ascii_case(&email))
        {
            return Err(PortalQuoteApiError::RecipientNotFound.into());
        }
        if !self.quotes_full_open(tenant).await? {
            return Err(Status::permission_denied(QUOTE_PURCHASE_ORDERS_NOT_IN_PLAN));
        }

        let _ = self
            .store
            .set_quote_purchase_order(quote_id, tenant, request.into_inner().purchase_order)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;

        Ok(Response::new(SetQuotePurchaseOrderResponse {}))
    }

    #[tracing::instrument(skip_all)]
    async fn add_quote_comment(
        &self,
        request: Request<AddQuoteCommentRequest>,
    ) -> Result<Response<AddQuoteCommentResponse>, Status> {
        let tenant = request.tenant()?;
        let quote_id = request.portal_resource()?.quote()?;
        let email = request.portal_resource()?.quote_recipient_email()?;
        if !self.quotes_full_open(tenant).await? {
            return Err(Status::permission_denied(QUOTES_FULL_NOT_IN_PLAN));
        }
        let origin = origin_of(&request);
        let inner = request.into_inner();
        let comment = self
            .store
            .add_quote_comment(
                tenant,
                QuoteCommentNew {
                    quote_id,
                    author: QuoteCommentAuthor::Recipient { email },
                    body: inner.body,
                    shared: true,
                    is_change_request: inner.request_changes,
                    anchor: inner.anchor,
                },
                origin,
            )
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;
        Ok(Response::new(AddQuoteCommentResponse {
            comment: Some(comment_to_proto(&comment, true)),
        }))
    }
}

/// The portal as `viewer_email` sees the quote: what GetQuotePortal serves and the seller's
/// preview shows. Reads only.
pub(crate) async fn quote_portal_details(
    store: &Store,
    services: &Services,
    links: &crate::quotes::QuoteFileLinks,
    documents: &dyn QuoteDocuments,
    features: &dyn Entitlements,
    tenant: TenantId,
    detailed_quote: DetailedQuote,
    viewer_email: String,
) -> Result<QuotePortalDetails, Status> {
    let pdf_url = documents
        .pdf_url(tenant, &detailed_quote.quote)
        .await
        .map_err(Into::<PortalQuoteApiError>::into)?;
    // Display-only: the quote still opens when its totals cannot be computed.
    let totals = match services.quote_totals(tenant, &detailed_quote).await {
        Ok(totals) => Some(crate::quotes::mapping::quotes::totals_to_proto(&totals)),
        Err(error) => {
            log::warn!(
                "no totals for quote {:?}: {error:?}",
                detailed_quote.quote.id
            );
            None
        }
    };

    let coupon_terms = detailed_quote
        .coupon_terms()
        .iter()
        .map(crate::quotes::mapping::quotes::coupon_terms_to_proto)
        .collect::<Vec<_>>();

    // Get customer
    let customer = detailed_quote.customer.clone();

    // Get existing signatures
    let signatures = detailed_quote.signatures;

    // Get recipients
    let recipients = &detailed_quote.quote.recipients;

    // Map recipients with signature status
    let recipients_with_status: Vec<QuoteRecipient> = recipients
        .iter()
        .map(|r| QuoteRecipient {
            email: r.email.clone(),
            name: r.name.clone(),
            title: None,
            has_signed: signatures.iter().any(|s| {
                s.signer_role == QuoteSignerRoleEnum::Customer
                    && s.signed_by_email.eq_ignore_ascii_case(&r.email)
            }),
        })
        .collect();

    let countersigners: Vec<QuoteCountersignerStatus> = detailed_quote
        .quote
        .countersigners
        .iter()
        .map(|c| QuoteCountersignerStatus {
            name: c.name.clone(),
            title: c.title.clone(),
            has_signed: signatures.iter().any(|s| {
                s.signer_role == QuoteSignerRoleEnum::Countersigner
                    && s.signed_by_email.eq_ignore_ascii_case(&c.email)
            }),
            is_viewer: c.email.trim().eq_ignore_ascii_case(viewer_email.trim()),
        })
        .collect();

    let proto_signatures: Vec<QuoteSignature> = signatures
        .into_iter()
        .map(|s| QuoteSignature {
            id: s.id.as_proto(),
            signed_by_name: s.signed_by_name,
            // The seller's countersigners stay anonymous to the customer.
            signed_by_email: if s.signer_role == QuoteSignerRoleEnum::Countersigner
                && !s.signed_by_email.eq_ignore_ascii_case(&viewer_email)
            {
                String::new()
            } else {
                s.signed_by_email
            },
            signed_by_title: s.signed_by_title,
            signed_at: s.signed_at.as_proto(),
            signature_method: s.signature_method,
            signer_role: crate::quotes::mapping::quotes::signer_role_to_proto(s.signer_role) as i32,
            revision: s.revision,
        })
        .collect();

    let current_recipient_email = viewer_email.clone();
    let (current_recipient_name, current_signer_role) = recipients
        .iter()
        .find(|r| r.email.eq_ignore_ascii_case(&current_recipient_email))
        .map(|r| (r.name.clone(), QuoteSignerRoleEnum::Customer))
        .or_else(|| {
            detailed_quote
                .quote
                .countersigners
                .iter()
                .find(|c| !c.automatic && c.email.eq_ignore_ascii_case(&current_recipient_email))
                .map(|c| (c.name.clone(), QuoteSignerRoleEnum::Countersigner))
        })
        .ok_or(PortalQuoteApiError::RecipientNotFound)?;

    let code_verified = store
        .quote_signature_code_verified(tenant, detailed_quote.quote.id, &current_recipient_email)
        .await
        .map_err(Into::<PortalQuoteApiError>::into)?;

    // Map quote to proto
    let mut proto_quote = crate::quotes::mapping::quotes::quote_to_proto(
        &detailed_quote.quote,
        Some(detailed_quote.customer.name.clone()),
        true,
    );
    crate::quotes::link_attachments(documents, tenant, detailed_quote.quote.id, &mut proto_quote)?;
    let has_attachments = !detailed_quote.quote.attachments.is_empty();

    // Map customer to proto
    let proto_customer =
        crate::customers::mapping::customer::ServerCustomerWrapper::try_from(customer)
            .map(|v| v.0)
            .map_err(Into::<PortalQuoteApiError>::into)?;

    let proto_components = detailed_quote
        .components
        .iter()
        .map(|c| {
            crate::quotes::mapping::quotes::quote_component_to_proto(
                c,
                &detailed_quote.quote.currency,
            )
        })
        .collect::<Vec<_>>();

    let proto_add_ons = detailed_quote
        .add_ons
        .iter()
        .map(crate::quotes::mapping::quotes::quote_add_on_to_proto)
        .collect::<Vec<_>>();

    let entity = Some(
        crate::invoicingentities::mapping::invoicing_entities::domain_to_public_proto(
            detailed_quote.invoicing_entity.clone(),
        ),
    );

    // Effective minimum-commitment shown on the signed document: the quote-level override
    // when set, otherwise the plan's default floor (the term that applies at conversion).
    let minimum_commitment = match &detailed_quote.minimum {
        Some(quote_min) => Some(crate::plans::mapping::plans::quote_minimum_to_proto(
            quote_min,
        )),
        None => store
            .get_plan_minimum(detailed_quote.quote.plan_version_id, tenant)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?
            .as_ref()
            .map(crate::plans::mapping::plans::minimum_commitment_to_proto),
    };

    let branding = crate::portal::branding::resolve_portal_branding(
        store,
        tenant,
        &detailed_quote.invoicing_entity,
    )
    .await
    .map_err(Into::<PortalQuoteApiError>::into)?;
    // The entity's logo, else the default entity's, as the customer portal shows it.
    let logo_url = branding.logo_attachment_id.map(|id| links.logo(id));
    let show_powered_by = !meteroid_app::features::is_whitelabel(features, store, tenant).await;

    let full_open = tenant_gate_open(features, store, tenant, gates::QUOTES_FULL)
        .await
        .map_err(Into::<PortalQuoteApiError>::into)?;
    let customer_on_pending = current_signer_role == QuoteSignerRoleEnum::Customer
        && detailed_quote.quote.status == QuoteStatusEnum::Pending;
    let discussion_open = full_open && customer_on_pending;
    let comments = store
        .list_quote_comments(tenant, detailed_quote.quote.id, true)
        .await
        .map_err(Into::<PortalQuoteApiError>::into)?
        .iter()
        .map(|c| comment_to_proto(c, true))
        .collect();

    Ok(QuotePortalDetails {
        quote: Some(proto_quote),
        customer: Some(proto_customer),
        entity,
        components: proto_components,
        add_ons: proto_add_ons,
        signatures: proto_signatures,
        recipients: recipients_with_status,
        current_recipient_email,
        current_recipient_name,
        logo_url,
        minimum_commitment,
        purchase_order_enabled: full_open,
        discussion_enabled: discussion_open && detailed_quote.quote.allow_questions,
        change_requests_enabled: discussion_open && detailed_quote.quote.allow_change_requests,
        decline_enabled: customer_on_pending && detailed_quote.quote.allow_decline,
        theme_mode: branding.theme_mode,
        roundness: branding.roundness,
        show_powered_by,
        totals,
        comments,
        coupon_terms,
        current_signer_role: crate::quotes::mapping::quotes::signer_role_to_proto(
            current_signer_role,
        ) as i32,
        countersigners,
        code_verified,
        default_consent_text: if has_attachments {
            DEFAULT_CONSENT_TEXT_WITH_ATTACHMENTS
        } else {
            DEFAULT_CONSENT_TEXT
        }
        .to_string(),
        pdf_url,
    })
}

impl PortalQuoteServiceComponents {
    async fn quotes_full_open(&self, tenant: TenantId) -> Result<bool, Status> {
        tenant_gate_open(
            self.feature_resolver.as_ref(),
            &self.store,
            tenant,
            gates::QUOTES_FULL,
        )
        .await
        .map_err(|e| Into::<PortalQuoteApiError>::into(e).into())
    }
}

/// How a signer signs in the portal: drawn (`digital_signature` from the web portal, or `drawn`) or
/// typed. The seller's countersignature methods are never accepted from here.
const PORTAL_SIGNATURE_METHODS: [&str; 3] = ["digital_signature", "drawn", "typed"];

pub(crate) const DEFAULT_CONSENT_TEXT: &str = "I have read this quote and accept it, with its terms and pricing, on behalf of my organization.";
pub(crate) const DEFAULT_CONSENT_TEXT_WITH_ATTACHMENTS: &str = "I have read this quote and its attached documents and accept them, with their terms and pricing, on behalf of my organization.";

pub(crate) fn origin_of<T>(request: &Request<T>) -> RequestOrigin {
    let context = meteroid_app::security_context::from_request(request);
    RequestOrigin {
        ip_address: context.ip_address,
        user_agent: context.user_agent,
    }
}
