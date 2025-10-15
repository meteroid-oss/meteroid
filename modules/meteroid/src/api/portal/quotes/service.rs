use crate::api::portal::quotes::PortalQuoteServiceComponents;
use crate::api::portal::quotes::error::PortalQuoteApiError;
use crate::api::shared::conversions::ProtoConv;
use crate::services::storage::Prefix;
use chrono::Utc;
use common_grpc::middleware::server::auth::RequestExt;
use meteroid_grpc::meteroid::portal::quotes::v1::portal_quote_service_server::PortalQuoteService;
use meteroid_grpc::meteroid::portal::quotes::v1::{
    GetQuotePortalRequest, GetQuotePortalResponse, QuotePortalDetails, QuoteRecipient,
    QuoteSignature, SetQuotePurchaseOrderRequest, SetQuotePurchaseOrderResponse, SignQuoteRequest,
    SignQuoteResponse,
};
use meteroid_store::domain::QuoteSignatureNew;
use meteroid_store::domain::enums::QuoteStatusEnum;
use meteroid_store::repositories::quotes::QuotesInterface;
use std::time::Duration;
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

        // Get quote details
        let detailed_quote = self
            .store
            .get_detailed_quote_by_id(tenant, quote_id)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;

        // Check if quote is still valid
        if let Some(expires_at) = detailed_quote.quote.expires_at
            && expires_at < Utc::now().naive_utc()
        {
            return Err(PortalQuoteApiError::QuoteExpired.into());
        }

        // Get customer
        let customer = detailed_quote.customer.clone();

        // Get logo URL if available
        let logo_url =
            if let Some(logo_attachment_id) = detailed_quote.invoicing_entity.logo_attachment_id {
                self.object_store
                    .get_url(
                        logo_attachment_id,
                        Prefix::ImageLogo,
                        Duration::from_secs(7 * 86400),
                    )
                    .await
                    .map_err(Into::<PortalQuoteApiError>::into)?
            } else {
                None
            };

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
                has_signed: signatures.iter().any(|s| s.signed_by_email == r.email),
            })
            .collect();

        // Map signatures to proto
        let proto_signatures: Vec<QuoteSignature> = signatures
            .into_iter()
            .map(|s| QuoteSignature {
                id: s.id.as_proto(),
                signed_by_name: s.signed_by_name,
                signed_by_email: s.signed_by_email,
                signed_by_title: s.signed_by_title,
                signed_at: s.signed_at.as_proto(),
                signature_method: s.signature_method,
            })
            .collect();

        // Get current recipient info from token
        let current_recipient_email = request.portal_resource()?.quote_recipient_email()?;
        let current_recipient = recipients
            .iter()
            .find(|r| r.email == current_recipient_email)
            .ok_or_else(|| PortalQuoteApiError::RecipientNotFound)?;

        // Map quote to proto
        let proto_quote = crate::api::quotes::mapping::quotes::quote_to_proto(
            &detailed_quote.quote,
            Some(detailed_quote.customer.name.clone()),
            true,
        );

        // Map customer to proto
        let proto_customer =
            crate::api::customers::mapping::customer::ServerCustomerWrapper::try_from(customer)
                .map(|v| v.0)
                .map_err(Into::<PortalQuoteApiError>::into)?;

        let proto_components = detailed_quote
            .components
            .iter()
            .map(crate::api::quotes::mapping::quotes::quote_component_to_proto)
            .collect::<Vec<_>>();

        let entity = Some(
            crate::api::invoicingentities::mapping::invoicing_entities::domain_to_public_proto(
                detailed_quote.invoicing_entity.clone(),
            ),
        );

        Ok(Response::new(GetQuotePortalResponse {
            quote: Some(QuotePortalDetails {
                quote: Some(proto_quote),
                customer: Some(proto_customer),
                entity,
                components: proto_components,
                signatures: proto_signatures,
                recipients: recipients_with_status,
                current_recipient_email,
                current_recipient_name: current_recipient.name.clone(),
                logo_url,
            }),
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

        // Get IP and User-Agent from request headers
        let headers = request.metadata();
        let ip_address = headers
            .get("x-forwarded-for")
            .or_else(|| headers.get("x-real-ip"))
            .and_then(|v| v.to_str().ok())
            .map(std::string::ToString::to_string);

        let user_agent = headers
            .get("user-agent")
            .and_then(|v| v.to_str().ok())
            .map(std::string::ToString::to_string);

        let inner = request.into_inner();

        // Validate that the signing email matches the token's recipient email
        if inner.recipient_email != token_recipient_email {
            return Err(PortalQuoteApiError::InvalidArgument(
                "Signing email must match the token recipient email".to_string(),
            )
            .into());
        }

        // Get quote to verify status
        let quote = self
            .store
            .get_quote_by_id(tenant, quote_id)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;

        // Check if quote is in a signable state
        if quote.status != QuoteStatusEnum::Pending {
            return Err(PortalQuoteApiError::NotSignable.into());
        }

        // Check if quote has expired
        if let Some(expires_at) = quote.expires_at
            && expires_at < Utc::now().naive_utc()
        {
            return Err(PortalQuoteApiError::QuoteExpired.into());
        }

        // Parse recipients and verify the signer is a valid recipient
        let recipients = &quote.recipients;

        let is_valid_recipient = recipients.iter().any(|r| r.email == inner.recipient_email);

        if !is_valid_recipient {
            return Err(PortalQuoteApiError::RecipientNotFound.into());
        }

        // Check if already signed by this email
        let existing_signatures = self
            .store
            .list_quote_signatures(quote_id)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;

        if existing_signatures
            .iter()
            .any(|s| s.signed_by_email == inner.recipient_email)
        {
            return Err(PortalQuoteApiError::AlreadySigned.into());
        }

        // Create signature
        let signature = QuoteSignatureNew {
            quote_id,
            signed_by_name: inner.signed_by_name.clone(),
            signed_by_email: inner.recipient_email.clone(),
            signed_by_title: if inner.signed_by_title.is_empty() {
                None
            } else {
                Some(inner.signed_by_title.clone())
            },
            signature_data: Some(inner.signature_data),
            signature_method: inner.signature_method,
            ip_address: ip_address.clone(),
            user_agent: user_agent.clone(),
            verification_token: None,
        };

        // Save signature
        let inserted_signature = self
            .store
            .insert_quote_signature(signature)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;

        // Check if all recipients have signed
        let updated_signatures = self
            .store
            .list_quote_signatures(quote_id)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;

        let all_signed = recipients.iter().all(|r| {
            updated_signatures
                .iter()
                .any(|s| s.signed_by_email == r.email)
        });

        // If all recipients have signed, update quote status to Accepted
        if all_signed {
            self.store
                .accept_quote(quote_id, tenant)
                .await
                .map_err(Into::<PortalQuoteApiError>::into)?;
        }

        Ok(Response::new(SignQuoteResponse {
            success: true,
            signature_id: inserted_signature.id.as_proto(),
        }))
    }

    #[tracing::instrument(skip_all)]
    async fn set_quote_purchase_order(
        &self,
        request: Request<SetQuotePurchaseOrderRequest>,
    ) -> Result<Response<SetQuotePurchaseOrderResponse>, Status> {
        let tenant = request.tenant()?;
        let quote_id = request.portal_resource()?.quote()?;

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

        let _ = self
            .store
            .set_quote_purchase_order(quote_id, tenant, request.into_inner().purchase_order)
            .await
            .map_err(Into::<PortalQuoteApiError>::into)?;

        Ok(Response::new(SetQuotePurchaseOrderResponse {}))
    }
}
